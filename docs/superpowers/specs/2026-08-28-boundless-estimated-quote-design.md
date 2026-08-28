# Boundless Estimated Quote Design

## Goal

Allow the RISC0 Boundless proposal and aggregation paths to construct the expected journal and
estimate request cycles without first executing the guest locally. This removes the local dry-run
from the normal Boundless submission path while retaining an explicit evaluated strategy and a
bounded fallback for rare numeric failures.

This change affects request pricing and timeout metadata only. Boundless still proves the original
guest program over the original input, and the existing fulfillment verification remains the source
of truth for proof validity.

## Configuration

Replace the misleading `raiko_agent` quote strategy with `estimated`:

```toml
[prover.risc0.boundless.batch_quote]
strategy = "estimated"

[prover.risc0.boundless.aggregation_quote]
strategy = "estimated"
```

`QuoteSizing` contains `Estimated`, `Evaluated`, and `Fixed { mcycles }`. `Estimated` is the new
default. The serialized name `raiko_agent` remains an alias for `Estimated` so an existing explicit
configuration continues to deserialize, but it no longer retains a separate rounding codepath.
Documentation and generated examples expose only `estimated`.

Only `Estimated` bypasses local execution. `Evaluated` keeps the exact local dry-run. `Fixed` keeps
its current behavior, including local execution to obtain the journal, so this change does not
silently alter that strategy beyond removing the retired enum variant.

## Request Metadata Preparation

Add `crates/prover/src/boundless/estimation.rs` as the authoritative implementation for estimated
cycles and deterministic journals. `boundless/mod.rs` chooses one of two paths before constructing a
Boundless request:

1. `Estimated` decodes and validates the stage input, derives its journal, and estimates mcycles.
2. `Evaluated` or `Fixed` runs the existing local RISC0 executor, obtains actual mcycles and journal,
   and applies the configured quote strategy.

An estimated request records `quoted_mcycles_count = Some(estimate)` and
`evaluated_mcycles_count = None`. A locally executed request, including an estimation fallback,
records both fields. No new persistence field or checkpoint schema version is required because the
evaluated field is already optional.

## Proposal Cycle Estimate

The proposal estimator reads each post-Unzen block zkGas from
`GuestInput.witnesses[].block.header.difficulty`, rejects an empty witness/block set, and computes:

```text
mcycles = ceil(
    511.8367085993759
    + 0.000003714503729246405 * total_zkgas
    + 2.2737130481392764 * block_count
)
```

These M2 coefficients were fitted on the completed 80-sample Hoodi fit split; the separate 40-sample
Hoodi calibration split was not used to change them. They were then evaluated against the available
20-sample Mainnet holdout. On that Mainnet set, MAPE was 5.87 percent and 19 of 20 samples were
within 10 percent. The estimate is used directly; the retired 1,000-mcycle bucket and 2,000-mcycle
floor are not applied.

Production code uses checked integer arithmetic rather than floating point. With a scale of
`1_000_000_000_000`, the numerator is:

```text
511_836_708_599_376
+ 3_714_504 * total_zkgas
+ 2_273_713_048_139 * block_count
```

The result is divided by the scale with ceiling. Every conversion, sum, and multiplication is
checked, and the final value must fit a positive `u32` mcycle count.

## Aggregation Cycle Estimate

RISC0 aggregation cost is dominated by verifying each child receipt. Estimate it as:

```text
aggregation_mcycles = 180 * child_receipt_count
```

The 180-mcycle value is a deliberately simple per-child estimate. As a current-data check, the
available Mainnet runtime snapshot contains 80 five-child RISC0 aggregations at 817 or 818 actual
mcycles; the estimate is 900 mcycles, approximately ten percent conservative. The input must contain
at least one receipt, and receipt and carry-data counts must match. Multiplication and conversion are
checked.

## Deterministic Journals

### Proposal

The proposal guest commits the Shasta subproof input hash. After non-panicking validation of the
carry fields required by the hash encoding, the host journal is:

```text
hash_shasta_subproof_input(guest_input.proof_carry_data)
```

The resulting journal is exactly 32 bytes. This derives the public output only; it does not replace
guest execution or claim that the proposal input is valid.

### Aggregation

Decode `ShastaRisc0AggregationGuestInput`, validate the non-empty and equal-length receipt/carry
vectors, require the zero ZK prover address, and bounds/sequence-validate the carry data. Derive each
child public input as `hash_shasta_subproof_input(carry)`, convert the proposal image ID using the
same little-endian representation as the guest, and invoke the shared Shasta aggregation-output
logic with a no-op proof-verification callback.

The host does not deserialize or verify child receipts merely to derive the expected journal. The
RISC0 aggregation guest still verifies every receipt against the proposal image ID and checks its
journal against the corresponding carry data. A bad receipt therefore cannot produce a fulfillable
proof and does not weaken proof verification.

## Errors and Fallback

Structural errors fail immediately:

- proposal or aggregation input cannot be deserialized;
- proposal has no witness/block;
- aggregation has no child proof;
- aggregation receipt and carry counts differ;
- carry data, prover address, or aggregation linkage is invalid;
- a derived journal has an unexpected length.

Rare numeric estimation failures emit a warning and fall back to the existing local execute path:

- a proposal zkGas value is zero or cannot be represented by the estimator;
- total zkGas, a scaled model term, or the final proposal estimate overflows;
- aggregation child-count multiplication or final conversion overflows.

If local execution also fails, its error is returned. A successful fallback uses the actual local
mcycles and journal and records `evaluated_mcycles_count = Some(actual)`.

## Verification

Focused regression coverage must establish:

- `estimated` and the legacy serialized alias deserialize to the same strategy;
- `raiko_agent` is no longer a distinct implementation path;
- checked proposal arithmetic matches the decimal M2 formula on all collected Mainnet samples;
- empty input and malformed structure fail directly;
- zero/oversized zkGas and arithmetic overflow select the local fallback;
- proposal journal derivation matches the RISC0 guest journal for a valid fixture;
- aggregation estimation scales with child count and rejects zero/mismatched vectors;
- aggregation journal derivation matches the RISC0 guest journal for valid receipt-backed input;
- estimated progress omits `evaluated_mcycles_count`, while evaluated and fallback progress retain it;
- proposal and aggregation fulfillment validation remains unchanged.

Because journal equality is proof-interface behavior, the complete change requires independent
adversarial review and independent behavioral verification before it is ready for a PR.

## Non-Goals

- No Boundless proof or auction submission is needed to calibrate or test the estimator.
- Do not change the zkGas protocol schedule or guest programs.
- Do not add a second boolean feature flag or a network-specific coefficient.
- Do not infer aggregation cycles from proposal zkGas.
- Do not remove the explicit `evaluated` or `fixed` strategies.
