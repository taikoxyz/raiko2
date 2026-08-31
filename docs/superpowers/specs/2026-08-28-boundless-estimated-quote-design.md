# Boundless Estimated Quote Design

## Goal

Allow an explicitly configured RISC0 Boundless proposal or aggregation path to construct the
expected journal and estimate request cycles without first executing the guest locally. This removes
the local dry-run from that opt-in path while retaining explicit evaluated and fixed strategies and
a bounded local-execution fallback outside the estimator's operating policy.

This change affects request pricing and timeout metadata only. Boundless still proves the original
guest program over the original input, and the existing fulfillment verification remains the source
of truth for proof validity.

## Configuration

Use `estimated` as an explicit quote strategy:

```toml
[prover.risc0.boundless.batch_quote]
strategy = "estimated"

[prover.risc0.boundless.aggregation_quote]
strategy = "estimated"
```

`QuoteSizing` contains only `Estimated`, `Evaluated`, and `Fixed { mcycles }`. Remove the
`RaikoAgent` variant together with its batch/aggregation rounding constants, helpers, tests, and
documentation. The serialized value `strategy = "raiko_agent"` is not an alias; it fails
deserialization with a clear list of supported strategies so an old explicit configuration cannot
silently change behavior.

Selecting `Estimated` is also the release operator's assertion that the deployed guest and RISC0
runtime remain compatible with the committed calibration. The request path does not pin or compare
an ELF hash, image ID, source revision, or RISC0 SDK version. Those values remain model provenance
for release review, not runtime gates. A release whose execution behavior has materially changed
must use `Evaluated` until the operator has checked and, if necessary, refreshed the model artifact.

Only `Estimated` bypasses local execution. `Evaluated` keeps the exact local dry-run. `Fixed` keeps
its current behavior, including local execution to obtain the journal. To avoid silently choosing a
replacement for an omitted strategy, an explicitly supplied Boundless configuration must contain
both `batch_quote` and `aggregation_quote`. Remove the field-level serde defaults for those two
tables. The internal `BoundlessConfig::default()` uses `Evaluated` only when the entire inactive
Boundless configuration is omitted; per-network overrides remain optional and inherit the explicit
base strategies.

## Request Metadata Preparation

Add `crates/prover/src/boundless/estimation.rs` as the evaluator and deterministic-journal module.
It compile-time embeds `crates/prover/models/risc0-zkgas.json` with `include_str!`,
deserializes and validates it once when an `Estimated` strategy is configured, and exposes typed
estimation results to `boundless/mod.rs`. The JSON artifact is the single source of truth for model
IDs, calibration provenance, coefficients, execution configuration, operating policy, and
calibrated aggregation counts. `estimation.rs` contains no duplicated constants for those values;
it owns only schema validation, checked arithmetic, policy checks, and journal derivation. An
invalid embedded artifact is a build/configuration defect and rejects `Estimated` during startup
rather than falling back at request time. Artifact validation checks its internal schema and values;
it does not compare its release-provenance fields with the running binary.

`boundless/mod.rs` chooses one of two paths before constructing a Boundless request:

1. `Estimated` decodes and validates the stage input, derives its journal, and estimates mcycles.
2. `Evaluated` or `Fixed` runs the existing local RISC0 executor, obtains actual mcycles and journal,
   and applies the configured quote strategy.

For `Estimated`, clone the SDK `StandardRequestBuilder` for that request, replace the clone's public
`preflight_layer` with `Default::default()`, and set its public `skip_preflight` field to `Some(true)`.
This is not a new raiko2 TOML option: selecting `Estimated` implies both SDK changes. Merely cloning
the builder is insufficient because the cloned `PreflightLayer` contains a `LocalExecutor` whose
cache state is shared through `Arc`. The default replacement has a fresh executor cache and no
downloader. Build the request through this isolated builder directly; calling
`client.build_request(...)` would select the client's shared builder and defeat the isolation.

Raiko2 already supplies `request_input`, `cycles`, `journal`, and `image_id`. The replacement
preflight layer therefore accepts those values without guest execution; its best-effort URL-input
cache fill cannot download the just-uploaded input because the replacement has no downloader, and
it cannot insert estimated data into the shared client's executor cache. `skip_preflight` separately
disables the later requestor pricing check. The request-scoped replacement leaves the shared client
and the `Evaluated`/`Fixed` preflight cache and pricing checks unchanged. An estimation fallback
still uses this isolated builder after raiko2 has performed its one intentional local execution,
preventing an SDK-side download, cache mutation, or second execution.

An estimated request records `quoted_mcycles_count = Some(estimate)` and
`evaluated_mcycles_count = None`. A locally executed request, including an estimation fallback,
records both fields. Progress also records the selected quote strategy and, for an estimate, its
model ID.

The runtime already persists quoted and evaluated counts, but `BoundlessSubmissionResume` currently
drops them when reconstructing a checkpoint. Extend the backward-compatible resume payload with
quote counts, strategy, and model ID. These fields form the quote context of one request-ID lineage.
Polling an already submitted request and constructing any rebid that reuses its request ID must use
the persisted quote context, even if configuration or the embedded model changed after restart.
Only a retry that rotates to a new request ID may compute a quote context from the current strategy
and model. Therefore every rung that can fulfill one request ID has one unambiguous quote
provenance.

The added fields deserialize as optional for checkpoint compatibility. Existing runtime metadata
already carries quoted and evaluated counts and must copy them into the resume payload. A legacy
checkpoint with a stored quote but no strategy/model keeps that exact quote and reports unavailable
provenance. If a legacy checkpoint lacks the quote itself, the service may poll its existing request
but must fail closed instead of submitting a same-ID rebid; it may use current configuration only
after the old request becomes terminal and rotation produces a new ID. The request digest, image
reference, exact maximum price, and deadlines remain the authoritative recovery identity.

## Configuration Migration

Removing `raiko_agent` is an intentional breaking configuration cleanup. Before deploying a binary
with this change, every enabled Boundless environment must explicitly select `estimated`,
`evaluated`, or `fixed` for both stages. The Mainnet and Hoodi deployment configurations inspected
during this design select `evaluated`, but their deployment repositories remain the source of truth
and must be rechecked immediately before rollout. Legacy `raiko2-k8s` Hoodi/Tolba configurations
that still name `raiko_agent` must move to `evaluated` before they consume the new binary. The
Masaya configurations this design also listed are moot since raiko2 #242 removed Masaya support.

For each later guest or RISC0 runtime release, the release owner must decide explicitly whether the
existing calibration is still applicable. Keeping `estimated` means accepting that compatibility;
switching the affected stage to `evaluated` is the safe rollout choice while measurements are being
refreshed. Raiko2 does not make this release decision from embedded ELF or dependency identifiers.

The schema-v2 artifact introduced by this change intentionally retains the measured pre-#242
proposal ELF `d7a4aca3769005d30772a6a1d4c47c95f7d6692244a3b017b181935a855e6b35`. It was not
recalibrated against the post-#242 `main` ELF or the v0.6.0 release guest. Enabling `estimated` with
either different ELF therefore accepts unmeasured cycle drift for quote price and timeout sizing;
it does not affect proof validity. The exact calibration identity remains recorded for audit, but it
is deliberately not a runtime availability gate.

This repository updates `config.example.toml`, `docs/API.md`, and `docs/operations.md` to expose the
three supported values and the required-stage-table rule. External configuration validators must
accept `estimated` and reject `raiko_agent` in coordinated follow-up changes. This repository does
not perform any deployment or rollout.

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
Hoodi calibration split was not used to change them. The original experiment selected M1 before
looking at Mainnet, and its strict zero-underquote decision correctly recommended retaining local
pre-execution. M2 was selected later after inspecting the 20 Mainnet samples under a different
product requirement: approximate auction cost and per-mcycle deadlines within about ten percent.
Those 20 samples are therefore an evaluation set, not an untouched holdout.

On the 40-row Hoodi calibration split, the continuous fitted M2 model produced 17 underquotes,
0.094557 percent MAPE, and a maximum absolute error of 0.279512 percent, which was also the maximum
underquote; no row exceeded ten-percent absolute error. Applying the production scaled-integer
coefficients and final ceiling produced 12 underquotes, 0.093492 percent MAPE, and a maximum
absolute error and underquote of 0.264550 percent; again, no row exceeded ten-percent absolute
error.

On the Mainnet evaluation set, the continuous fitted M2 model had 5.87 percent MAPE. Nineteen of 20
predictions were below actual cycles; the largest underquote was 108.59 mcycles, or 5.75 percent.
Nineteen of 20 absolute errors were within ten percent; the remaining sample was a 21.94-percent
overquote. Applying the scaled-integer coefficients and final ceiling used by production gives 19
underquotes, 5.8422 percent MAPE, a maximum 108-mcycle or 5.7234-percent underquote, and a
547-mcycle or 21.9679-percent overquote for the isolated sample. The current publication gate is
empirical: every concrete fit, calibration, or evaluation observation admitted by the production
policy must be within ten-percent absolute error. The global `500_000_000` zkGas cap excludes the
isolated overquote, so all 19 admitted Mainnet evaluation samples pass that gate. A new untouched
holdout and the original zero-underquote gate are not prerequisites for this explicitly enabled
strategy, and the design does not claim that they passed.

### Accepted Approximation Contract

`Estimated` intentionally returns a coarse pricing input rather than the exact local-execution cycle
count. Selecting it accepts both underquotes and overquotes, including mismatches for network and
block-count combinations outside the collected sample rectangles. The ten-percent gate above is a
model publication and refresh check over concrete collected observations; it is not a mathematical
bound or per-request runtime guarantee for every future input admitted by the mechanical policy.

An admitted request does not execute the guest locally before quoting, so raiko2 cannot know that
request's actual error or fall back merely because its unknown error might exceed ten percent. The
runtime intentionally estimates every exact-Unzen request satisfying the documented
`execution_po2`, non-zero zkGas, checked-total, and numeric guards. Operators that require an exact
cycle count must select `evaluated`.

Implementation review should therefore treat the absence of a per-request ten-percent proof,
zero-underquote guarantee, untouched holdout, or per-network/per-block observed envelope as this
design's accepted product trade-off, not as a defect. Actionable blockers are a violation of the
mechanical admission or fallback contract, unsafe arithmetic, or a concrete newly collected
in-policy observation outside the publication error budget. The latter requires re-evaluating the
model, cap, or strategy before publishing that refresh; it does not retroactively turn historical
sample rectangles into runtime availability rules.

The estimate is used directly; no calibration margin, 1,000-mcycle bucket, or 2,000-mcycle floor is
applied. This is an explicit cost/latency trade-off: `with_cycles` is not a cryptographic execution
limit, but the value scales the configured price cap and per-mcycle lock/fulfillment deadlines. An
underestimate can therefore make an auction less attractive or expire it earlier. The accepted
approximately ten-percent error budget applies to those effects, not to proof validity.

Production code uses checked integer arithmetic rather than floating point. With a scale of
`1_000_000_000_000`, the numerator is:

```text
511_836_708_599_376
+ 3_714_504 * total_zkgas
+ 2_273_713_048_139 * block_count
```

The result is divided by the scale with ceiling. Every conversion, sum, and multiplication is
checked, and the final value must fit a positive `u32` mcycle count.

### Proposal Model Identity and Operating Policy

Commit a compact, reviewable runtime artifact at `crates/prover/models/risc0-zkgas.json`. It is
generated from the versioned fixture and policy under
`tests/fixtures/risc0-zkgas/2026-08-31-m2-global-cap-v2/` and records at least:

- content-addressed model ID and the originating experiment model ID;
- artifact schema version 2;
- proposal image ID `0xd6ab71c22201c23ef512b706f2e2d720f6da1b559fb76834aa9d4e35276f6e10`;
- RISC0 version `3.0.5`, calibrated `min_execution_po2 = 20`, source revision, ELF hash,
  generator-config hash,
  compact input-row hash, and validation-fixture hash;
- the decimal and scaled-integer coefficients;
- Hoodi fit/calibration and Mainnet evaluation counts and diagnostics;
- the fact that Mainnet influenced the M2 production choice and is not an untouched holdout;
- the global `max_total_zkgas = 500_000_000` operating cap.

The runtime intentionally uses a broad pricing policy rather than treating observed calibration
envelopes as hard availability boundaries:

```text
min_execution_po2: 20
max_total_zkgas: 500_000_000
```

Network name and block count do not gate proposal estimation. Block count remains a coefficient in
the M2 formula. Every witness must have non-zero zkGas, the checked total must be no more than the
global cap, and the input must contain at least one witness. Inputs above the cap fall back to local
execution. The committed Hoodi and Mainnet rows remain calibration and diagnostic evidence; they do
not define runtime min/max rectangles or a per-request accuracy guarantee.

Before estimating, a private `proposal_estimation_available(&GuestInput)` helper in
`boundless/estimation.rs` inspects every witness's `chain_spec.hard_forks` at that witness block's
number and timestamp. Estimation is available only when the highest active Taiko fork for every
block is exactly `TaikoFork::Unzen`; a pre-Unzen input or a future later Taiko fork therefore returns
unavailable. This is a Boundless-estimator implementation check, not a public validation API or a
model-policy field.

The request path also checks the artifact's execution minimum and global zkGas cap. An unavailable
fork, `execution_po2 < 20`, zero or over-cap zkGas, or numeric overflow emits a warning containing
the model ID and falls back to local execution. It deliberately does not compare the running
proposal image ID, ELF hash, source revision, or RISC0 SDK version with the artifact. Compatibility
of those release identities is reviewed when the deployment selects `estimated`.

### Validation Fixture

Commit `tests/fixtures/risc0-zkgas/2026-08-31-m2-global-cap-v2/validation.jsonl` with the 40 successful Hoodi
calibration rows and 20 successful Mainnet evaluation rows. The same versioned directory also keeps
the 80 Hoodi fit rows and explicit policy/provenance config used by the deterministic generator.
Each compact row contains
`network`, `split`, `proposal_id`, `block_count`, `total_zkgas`, and `actual_mcycles`; the Mainnet
rows use `split = "evaluation"` because they are no longer described as an untouched holdout. This
fixture is validation evidence, not a second runtime source for coefficients or operating policy.
The model artifact records its SHA-256 and the expected split counts.

Regression tests load the committed fixture and model artifact together, require the fixture hash,
schema, unique `(network, proposal_id)` keys, and exact 40/20 split counts to match, then recompute
the documented continuous-model and production-integer diagnostics. Tests must not depend on the
original machine-local collection directory. The reviewed generator config binds the raw cohort's
source/build/chain-spec hashes and fixed 10% policy, while the compact input hash binds every fitted
or evaluated numeric row. Later calibrations start with model ID `risc0-zkgas-m2-auto`; generation
replaces it in the committed fixture and runtime artifact with a 16-hex content-addressed suffix.

## Aggregation Cycle Estimate

RISC0 aggregation cost is dominated by verifying each child receipt. Estimate it as:

```text
aggregation_mcycles = 180 * child_receipt_count
```

The 180-mcycle value is a deliberately simple per-child estimate. Historical operator observation
puts a one-child aggregation near 175 mcycles. The available Mainnet runtime snapshot contains 80
five-child aggregations at 817 or 818 mcycles, for which the estimate is 900 mcycles. The snapshot's
deployed proposal image differs from the experiment proposal image, so these aggregation samples are
from a different release cohort. They are supporting evidence only; they do not calibrate the
current worktree aggregation ELF or prove a zero intercept.

Before enabling estimated aggregation, execute the current aggregation image locally with valid
receipt-backed inputs at child counts one through five and commit the image ID and results to the
model artifact. Enable `180 * child_receipt_count` only for counts whose measured absolute error and
underquote are within the accepted ten-percent budget. Counts outside the artifact's calibrated
set, including every count above five initially, fall back to local execution. As with proposal
estimation, the image ID is calibration provenance and is not compared at runtime. The release owner
must keep aggregation on `Evaluated` after a materially changed aggregation guest until the measured
counts have been checked or refreshed. The input must contain at least one receipt, and receipt and
carry-data counts must match. Multiplication and conversion are checked.

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

Estimation-policy and numeric failures emit a warning and fall back to the existing local execute
path:

- the private proposal availability check rejects the active Taiko fork;
- proposal `execution_po2` is below the calibrated minimum;
- any proposal witness has zero or unrepresentable zkGas, the checked total exceeds `500_000_000`,
  or the checked sum overflows;
- a scaled model term or the final proposal estimate overflows;
- aggregation child count is outside the calibrated set;
- aggregation child-count multiplication or final conversion overflows.

If local execution also fails, its error is returned. A successful fallback uses the actual local
mcycles and journal and records `quoted_mcycles_count = Some(actual)` and
`evaluated_mcycles_count = Some(actual)`.

## Verification

Focused regression coverage must establish:

- `raiko_agent` is rejected, its rounding implementation is absent, and `estimated` requires
  explicit selection;
- an operator-supplied Boundless configuration requires both stage quote tables, while an omitted
  inactive Boundless configuration retains the internal `Evaluated` default;
- `Estimated` uses a request-scoped SDK builder with a default replacement `preflight_layer` and
  `skip_preflight = Some(true)`, while `Evaluated` and `Fixed` retain the shared preflight layer and
  normal SDK pricing check;
- building an `Estimated` request does not download its just-uploaded URL input, invoke the local
  guest executor, or read or modify the shared client's executor cache;
- checked proposal arithmetic matches the decimal M2 formula across the committed Hoodi calibration
  and Mainnet evaluation rows;
- Hoodi continuous-M2 regression tests assert 17 underquotes, 0.094557-percent MAPE, a maximum
  0.279512-percent absolute error and underquote, and zero rows beyond ten-percent absolute error;
- Hoodi production-integer regression tests assert 12 underquotes, 0.093492-percent MAPE, a maximum
  0.264550-percent absolute error and underquote, and zero rows beyond ten-percent absolute error;
- Mainnet continuous-M2 regression tests assert 19 underquotes, 5.87-percent MAPE, a maximum
  observed 5.75-percent underquote, and the single 21.94-percent overquote;
- Mainnet production-integer regression tests assert 19 underquotes, 5.8422-percent MAPE, a maximum
  5.7234-percent underquote, and the single 21.9679-percent overquote;
- model artifact schema, execution minimum, and global zkGas-cap
  guards select either estimate or local fallback correctly;
- the private Boundless proposal availability check accepts inputs whose highest active Taiko fork
  is Unzen and selects local fallback for pre-Unzen or later-fork inputs;
- the committed validation fixture has its recorded hash and exact 40-row Hoodi calibration and
  20-row Mainnet evaluation splits, and both sets' documented diagnostics are reproducible from it;
- proposal and aggregation estimation do not compare the running ELF hash, image ID, source
  revision, or RISC0 SDK version with artifact provenance;
- the embedded JSON is the only source of model parameters, and malformed or internally
  inconsistent artifact data rejects `Estimated` configuration;
- network names and block counts do not gate estimation, `500_000_000` total zkGas is admitted, and
  `500_000_001` falls back;
- empty input and malformed structure fail directly;
- zero/oversized zkGas and arithmetic overflow select the local fallback;
- proposal journal derivation matches the RISC0 guest journal for a valid fixture;
- aggregation calibration records executions at child counts one through five before those counts
  are enabled, while counts outside the calibrated set fall back;
- aggregation estimation scales with calibrated child count and rejects zero/mismatched vectors;
- aggregation journal derivation matches the RISC0 guest journal for valid receipt-backed input;
- estimated progress omits `evaluated_mcycles_count`, while evaluated and fallback progress retain it;
- resumed submissions and same-ID rebids retain their persisted quote counts, strategy, and model
  ID across a simulated configuration/model change;
- a rebid uses current quote configuration only after rotating to a new request ID;
- a legacy resume without quote context cannot submit a same-ID rebid;
- proposal and aggregation fulfillment validation remains unchanged.

Because journal equality is proof-interface behavior, the complete change requires independent
adversarial review and independent behavioral verification before it is ready for a PR.

## Non-Goals

- No Boundless proof or auction submission is needed to calibrate or test the estimator.
- Do not change the zkGas protocol schedule or guest programs.
- Do not add a second user-visible feature/preflight flag or a network-specific coefficient.
- Do not infer aggregation cycles from proposal zkGas.
- Do not remove or silently reinterpret `evaluated` or `fixed` in this change.
- Do not add a runtime ELF, image-ID, source-revision, or RISC0-version compatibility gate; selecting
  `estimated` is a release decision.
