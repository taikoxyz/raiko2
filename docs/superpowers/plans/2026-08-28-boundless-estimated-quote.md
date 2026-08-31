# Boundless Estimated Quote Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan.

**Goal:** Add an opt-in Boundless `estimated` quote strategy that derives the RISC0 journal and cycle quote without local proposal/aggregation execution when the input satisfies the committed operating policy, while preserving one-execution local fallback and durable quote provenance.

**Architecture:** A new `boundless::estimation` module owns the embedded model schema, checked estimator arithmetic, operating-policy selection, fork guard, and deterministic proposal/aggregation journal construction. `boundless/mod.rs` turns each request into a durable `QuoteContext` before submission, uses a request-scoped isolated Boundless SDK builder for estimates and fallbacks, and preserves that context for every rung sharing a request ID. Configuration continues to choose the strategy per stage; the embedded JSON is the only runtime source for coefficients, the global zkGas cap, model identity, and calibrated aggregation counts.

**Tech Stack:** Rust 2024 workspace, serde/serde_json, bincode, RISC Zero 3.0.5, boundless-market 2.0.0, Tokio, TOML configuration, Python 3.11 experiment venv for fixture diagnostics.

**Spec:** `docs/superpowers/specs/2026-08-28-boundless-estimated-quote-design.md`

## Global Constraints

- Do not change guest source or generated ELFs.
- Do not add a public preflight flag, runtime ELF/image/version gate, or network-specific coefficient.
- Treat malformed stage input and invalid carry linkage as direct request errors. Treat operating-policy, execution-configuration, fork, zero/overflow, and uncalibrated-count failures as warning-plus-one-local-execution fallback.
- Use `apply_patch` for repository edits. Run Python through an existing virtual environment.
- Preserve unrelated worktree changes. Commit each coherent task only after its focused red/green checks pass.
- Before completion, run independent adversarial review and independent behavioral verification because journal equality, request pricing, and durable rebid state are cross-crate behavior.

### Task 1: Commit and validate the single-source model artifact

**Files:**

- Create: `crates/prover/models/risc0-zkgas.json`
- Create: `crates/prover/src/boundless/estimation.rs`
- Modify: `crates/prover/src/boundless/mod.rs`

**Interfaces:**

```rust
mod estimation;

pub(crate) struct EstimationModel(ValidatedModelArtifact);

pub(crate) fn estimation_model() -> Result<&'static EstimationModel, String>;
pub fn validate_estimation_model() -> Result<(), String>;
```

`estimation_model()` uses a `std::sync::OnceLock<Result<EstimationModel, String>>` and parses:

```rust
include_str!(concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/models/risc0-zkgas.json"
))
```

The artifact schema contains:

- `schema_version = 2`, content-addressed model ID `risc0-zkgas-m2-5adefe56336d7238`, and originating experiment model `M2`;
- provenance: source revision `4f8300497aba75605b9b8568b1955faa1f7f04bc`, proposal image ID `0xd6ab71c22201c23ef512b706f2e2d720f6da1b559fb76834aa9d4e35276f6e10`, proposal ELF SHA-256 `d7a4aca3769005d30772a6a1d4c47c95f7d6692244a3b017b181935a855e6b35`, RISC0 `3.0.5`, and `min_execution_po2 = 20`;
- generated config SHA-256, compact input-row SHA-256 `0cfbf1184483f2646eedb9833365e3f232bee9c68604ff94e2160949e8696328`, and validation fixture SHA-256 `dff36c84683011825a7372e43f846b678266f0f062515f44631922e9a7c47767`;
- decimal proposal coefficients and scaled integer coefficients with scale `1_000_000_000_000`;
- global proposal operating cap `max_total_zkgas = 500_000_000`; network and block count are not availability gates;
- exact cohort counts and documented diagnostics for Hoodi calibration and Mainnet evaluation;
- aggregation formula `per_child_mcycles = 180`, current aggregation image provenance, and the measured child-count rows produced in Task 7. The runtime calibrated set is derived only from rows with both absolute error and underquote within 10 percent.

Validation rejects unknown fields, wrong schema/model IDs, zero scale/coefficient/minimum-po2/cap values, diagnostics/count inconsistencies, invalid SHA-256/image formats, duplicated aggregation child counts, aggregation rows outside `1..=5`, and aggregation rows marked enabled outside the accepted error rule. Release provenance is validated for shape only and is not compared with the running binary.

**Step 1: Write failing schema tests**

Add unit tests in `estimation.rs` that parse a valid minimal artifact string and reject malformed JSON, missing fields, unknown fields, wrong model/schema IDs, invalid hashes, zero operating-policy values, and inconsistent aggregation calibration rows. Add a test proving production parameters are read from the parsed artifact rather than Rust constants.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::estimation::tests::model_
```

Expected: FAIL because the module, schema, and artifact do not exist.

**Step 2: Implement the private serde schema and validation**

Use private `#[serde(deny_unknown_fields)]` structs. Store decimal coefficients as strings for auditability and parse them only in fixture-regression tests; runtime arithmetic reads only the scaled integer fields from the artifact. Keep all coefficient/operating-policy/calibrated-count values out of Rust constants.

**Step 3: Add the artifact with proposal data and an initially empty aggregation calibration list**

The empty list means aggregation `Estimated` cannot estimate until Task 7 records current-image rows; it does not fall through to historical counts. Task 7 updates this same artifact before the full feature is considered ready.

**Step 4: Run the focused tests**

Run the command from Step 1.

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless/estimation.rs crates/prover/src/boundless/mod.rs crates/prover/models/risc0-zkgas.json
git commit -m "feat(boundless): embed quote estimation model"
```

### Task 2: Make quote strategy selection explicit and remove `raiko_agent`

**Files:**

- Modify: `crates/prover/src/boundless_config.rs`
- Modify: `bin/raiko2/src/config/prover.rs`
- Modify: `bin/raiko2/src/server/state/mod.rs`
- Modify: `config.example.toml`

**Interfaces:**

```rust
pub enum QuoteSizing {
    Estimated,
    Evaluated,
    Fixed { mcycles: u32 },
}
```

`QuoteSizing::default()` returns `Evaluated`. Remove `#[serde(default)]` from both quote fields in the prover-crate and binary-crate explicit `BoundlessConfig` structs. Retain `#[serde(default)]` on the outer inactive RISC0/Boundless config so omitting the entire table still constructs the internal evaluated default. `BoundlessPairConfig` quote fields remain optional and inherit the validated base value.

**Step 1: Write failing config tests**

Cover:

- `strategy = "estimated"` deserializes;
- `strategy = "raiko_agent"` fails and the error lists supported variants;
- an explicitly supplied Boundless table missing either stage quote fails;
- an omitted inactive Boundless table defaults both stages to `Evaluated`;
- per-pair omission inherits the base stage strategies;
- zero `Fixed` still fails;
- enabled Estimated invokes `raiko2_prover::boundless::validate_estimation_model()` during startup validation.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless_config
cargo test -p raiko2 config::prover
```

Expected: FAIL on the new strategy and required-field assertions.

**Step 2: Implement enum/default/serde changes and startup model validation**

Remove the legacy quote constants, rounding helpers, dispatch branches, and tests from `boundless/mod.rs`. In server startup validation, call the embedded-model validator only when an effective enabled Boundless pair selects Estimated for either stage. Do not compare running image/ELF/revision/RISC0 version.

**Step 3: Update the example config**

Set both base strategies explicitly to `evaluated`. Show `estimated` as an opt-in commented alternative and keep pair override examples optional.

**Step 4: Re-run focused tests and formatting**

```bash
cargo test -p raiko2-prover --features boundless boundless_config
cargo test -p raiko2 config::prover
cargo fmt --all -- --check
```

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless_config.rs crates/prover/src/boundless/mod.rs bin/raiko2/src/config/prover.rs bin/raiko2/src/server/state/mod.rs config.example.toml
git commit -m "refactor(boundless): require explicit quote strategies"
```

### Task 3: Implement proposal estimation and deterministic journal derivation

**Files:**

- Modify: `crates/prover/src/boundless/estimation.rs`
- Modify: `crates/prover/src/lib.rs`

**Interfaces:**

```rust
pub(crate) enum EstimateUnavailable {
    ExecutionPo2,
    Fork,
    Chain,
    Domain,
    ZeroZkGas,
    Numeric,
}

pub(crate) struct EstimatedRequestMetadata {
    pub model_id: String,
    pub mcycles: u32,
    pub journal: Vec<u8>,
}

pub(crate) fn estimate_proposal(
    input: &GuestInput,
    execution_po2: u32,
) -> RaikoResult<Result<EstimatedRequestMetadata, EstimateUnavailable>>;
```

Structural validation returns the outer `RaikoResult::Err`; domain/numeric rejection returns the inner `Err(EstimateUnavailable)` for local fallback.

**Step 1: Write failing proposal tests**

Build small in-memory `GuestInput` values and cover:

- empty witnesses is a direct error;
- a valid carry produces exactly the 32-byte `hash_shasta_subproof_input` journal after non-panicking carry validation;
- network names and observed block-count ranges do not gate estimation;
- total zkGas `500_000_000` estimates successfully and `500_000_001` is unavailable;
- `execution_po2 >= 20` is available and a lower value is unavailable;
- every witness must have highest active Taiko fork exactly Unzen; pre-Unzen is unavailable. Unit-test the ordered active-fork classifier with a private synthetic rank above Unzen so future Taiko enum variants cannot be accepted accidentally, without adding a production fork variant;
- zero difficulty and checked-add/multiply/final-conversion overflow are unavailable;
- the integer result is the ceiling of the artifact formula and fits positive `u32`.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::estimation::tests::proposal_
```

Expected: FAIL because the proposal estimator is absent.

**Step 2: Add safe carry validation before hashing**

Expose or reuse the shared Shasta carry-vector validation path so `hash_shasta_subproof_input` is never called on out-of-range uint48 fields. Do not duplicate proposal journal rules in `boundless/mod.rs`.

**Step 3: Implement fork/policy extraction and checked integer arithmetic**

Read each `witness.block.header.number`, `timestamp`, and `difficulty`; require exact Unzen and a non-empty witness list; sum non-zero `difficulty` through checked `u128` conversion/addition; require the total at or below the artifact cap; multiply and add all scaled terms with checked arithmetic; ceiling-divide by artifact scale; and convert to positive `u32`.

**Step 4: Re-run the focused tests**

Run the command from Step 1.

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless/estimation.rs crates/prover/src/lib.rs
git commit -m "feat(boundless): estimate proposal quote metadata"
```

### Task 4: Reproduce model diagnostics from the committed fixture

**Files:**

- Modify: `crates/prover/src/boundless/estimation.rs`
- Modify: `experiments/risc0-zkgas/tests/test_fit_model.py`

**Step 1: Write failing regression tests**

Rust tests load the artifact plus `tests/fixtures/risc0-zkgas/2026-08-31-m2-global-cap-v2/validation.jsonl`, require its SHA-256, six-field row schema, unique `(network, proposal_id)` keys, and exact 40 Hoodi calibration/20 Mainnet evaluation rows. Recompute both continuous and integer predictions and assert the artifact diagnostics with tight tolerances:

- Hoodi continuous: 17 underquotes, MAPE 0.094557%, maximum absolute/underquote 0.279512%, zero rows over 10%;
- Hoodi integer: 12 underquotes, MAPE 0.093492%, maximum absolute/underquote 0.264550%, zero rows over 10%;
- Mainnet continuous: 19 underquotes, MAPE 5.87%, maximum underquote 5.75%, one 21.94% overquote;
- Mainnet integer: 19 underquotes, MAPE 5.8422%, maximum underquote 5.7234%, one 21.9679% overquote.

The Python test independently checks the same committed artifact/fixture contract without reading `/tmp`.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::estimation::tests::fixture_
/path/to/venv/bin/python -m pytest experiments/risc0-zkgas/tests/test_fit_model.py -q
```

Expected: FAIL until the fixture loaders and artifact diagnostics are wired.

**Step 2: Implement diagnostics helpers under `#[cfg(test)]`**

Keep production runtime independent of the validation fixture. Parse decimal coefficients only in regression tests and compare the independently computed metrics with artifact fields.

**Step 3: Re-run tests**

Run the commands from Step 1.

Expected: PASS, with the Python experiment suite still reporting 23 passing tests or more after the added case.

**Step 4: Commit**

```bash
git add crates/prover/src/boundless/estimation.rs experiments/risc0-zkgas/tests/test_fit_model.py crates/prover/models/risc0-zkgas.json
git commit -m "test(boundless): lock quote model diagnostics"
```

### Task 5: Implement aggregation journal derivation and estimator guards

**Files:**

- Modify: `crates/prover/src/boundless/estimation.rs`
- Modify: `crates/prover/src/lib.rs`

**Interfaces:**

```rust
pub(crate) fn estimate_aggregation(
    encoded_input: &[u8],
) -> RaikoResult<Result<EstimatedRequestMetadata, EstimateUnavailable>>;
```

**Step 1: Write failing aggregation tests**

Cover:

- malformed bincode, zero children, receipt/carry length mismatch, non-zero prover address, invalid uint48 fields, and invalid carry sequence/linkage are direct errors;
- valid input derives each child input from the carry and computes exactly the same journal as `aggregate_shasta_zk_with_verifier` using a no-op verifier;
- proposal image words use `words_to_bytes_le`, matching the RISC0 guest;
- receipt bytes are not deserialized or verified by the host journal path;
- only child counts present in the artifact's accepted calibrated set estimate `180 * count`;
- uncalibrated count and checked multiplication/conversion overflow are unavailable.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::estimation::tests::aggregation_
```

Expected: FAIL because aggregation estimation is absent.

**Step 2: Reuse shared aggregation-output validation**

Construct `ShastaZkAggregationGuestInput` from carry-derived child hashes, then call `aggregate_shasta_zk_with_verifier(..., |_index, _input| Ok(()))`. This preserves one bounds/linkage source of truth and avoids receipt verification on the host.

**Step 3: Implement artifact-calibrated count selection and checked multiplication**

Do not admit counts from the historical five-child snapshot. An empty calibrated set always selects local fallback.

**Step 4: Re-run focused tests**

Run the command from Step 1.

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless/estimation.rs crates/prover/src/lib.rs
git commit -m "feat(boundless): derive aggregation quote metadata"
```

### Task 6: Introduce a durable quote context and one-execution preparation path

**Files:**

- Modify: `crates/prover/src/boundless/mod.rs`
- Modify: `crates/prover/src/lib.rs`

**Interfaces:**

```rust
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BoundlessQuoteStrategy {
    Estimated,
    Evaluated,
    Fixed,
}

struct QuoteContext {
    quoted_mcycles_count: u32,
    evaluated_mcycles_count: Option<u32>,
    strategy: Option<BoundlessQuoteStrategy>,
    model_id: Option<String>,
    journal: Vec<u8>,
    expected_input_hash: B256,
    isolated_builder: bool,
}
```

Progress and resume payloads add optional `quote_strategy` and `quote_model_id`; resume also adds optional quoted/evaluated counts. `Fixed` records `quoted = fixed` and `evaluated = actual`. Successful Estimated fallback records `quoted = actual`, `evaluated = actual`, strategy Estimated, and model ID `None`; the attempted model ID appears only in the fallback warning. Estimated success records `quoted = estimate`, `evaluated = None`.

**Step 1: Write failing quote-preparation tests**

Inject an execution closure/counter into a test-only metadata-preparation helper and cover:

- Estimated when available under the operating policy does not call local execution;
- Estimated unavailable calls local execution exactly once and quotes actual;
- Evaluated calls once and quotes actual;
- Fixed calls once for journal/evaluation and quotes fixed;
- malformed proposal/aggregation structure does not fall back;
- estimated and fallback contexts request SDK isolation; evaluated/fixed contexts do not;
- expected input hash comes from the derived/evaluated journal and retains existing fulfillment checks.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::tests::quote_context_
```

Expected: FAIL before the preparation helper exists.

**Step 2: Refactor `prove_boundless` inputs**

Pass a typed stage input alongside encoded bytes: proposal receives the already decoded `GuestInput`; aggregation receives the already built/decoded `ShastaRisc0AggregationGuestInput`. Perform metadata preparation before program upload and submission. Keep only one local executor call in all locally evaluated/fallback paths.

**Step 3: Replace the `(u32, u32)` telemetry tuples with optional evaluated counts**

Thread `&QuoteContext` or a compact serializable quote-provenance view through `FreshSubmissionContext`, `FulfillmentContext`, all progress persistence calls, tx-hash checkpoint updates, and stage metadata. Do not synthesize `evaluated = quoted` for successful estimates.

**Step 4: Re-run focused tests**

Run the command from Step 1 plus:

```bash
cargo test -p raiko2-prover --features boundless quoted_mcycles_count
```

Expected: the new tests PASS and the removed legacy rounding test filter runs zero tests.

**Step 5: Commit**

```bash
git add crates/prover/src/boundless/mod.rs crates/prover/src/lib.rs
git commit -m "feat(boundless): prepare estimated quote context"
```

### Task 7: Measure the current aggregation image at one through five children

**Files:**

- Create: `crates/prover/examples/boundless_aggregation_calibration.rs`
- Modify: `crates/prover/models/risc0-zkgas.json`
- Modify: `crates/prover/src/boundless/estimation.rs`

**Step 1: Implement a reproducible local calibration example**

The example loads the current fixed-path RISC0 Shasta aggregation ELF through `raiko2-guests`, computes its image ID, creates a structurally valid linked carry sequence and receipt-backed aggregation input for counts `1..=5`, executes only the aggregation guest with `execution_po2 = 20`, and prints stable JSON rows containing image ID, child count, actual user mcycles, predicted mcycles, signed error, absolute-error percent, and underquote percent.

Use RISC0 dev-mode assumption receipts whose claims contain the exact carry-derived 32-byte journals. This exercises the guest receipt-verification syscall/assumption path during execution; the example must reject a receipt whose claim or image ID does not match. It does not prove or submit anything to Boundless.

**Step 2: Run the calibration twice and compare exact cycles**

```bash
RISC0_DEV_MODE=1 cargo run -p raiko2-prover --example boundless_aggregation_calibration --features boundless
RISC0_DEV_MODE=1 cargo run -p raiko2-prover --example boundless_aggregation_calibration --features boundless
```

Expected: both runs report identical image ID and exact mcycle results for every count. If they differ, aggregation Estimated remains disabled by leaving `calibrated_counts` empty and the discrepancy is reported before proceeding.

**Step 3: Apply the fixed acceptance rule**

For each deterministic count, enable it only when `abs(predicted - actual) / actual <= 10%` and `max(actual - predicted, 0) / actual <= 10%`. Record all five measured rows and the aggregation image ID in the artifact; populate `calibrated_counts` with exactly the passing counts. Do not adjust `180` from these rows without a separate design revision.

**Step 4: Lock calibration regression tests**

Add tests that the artifact rows are unique and exactly cover `1..=5`, recompute each error, derive the calibrated set, and prove that the runtime set equals that derivation. The example output and artifact must use user mcycles with the same ceiling conversion as `evaluate_guest`.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::estimation::tests::aggregation_calibration_
```

Expected: PASS. If no count passes, proposal Estimated remains implementable but aggregation Estimated always falls back; document that limitation rather than admitting unsupported counts.

**Step 5: Commit**

```bash
git add crates/prover/examples/boundless_aggregation_calibration.rs crates/prover/src/boundless/estimation.rs crates/prover/models/risc0-zkgas.json
git commit -m "test(boundless): calibrate aggregation cycle estimate"
```

### Task 8: Isolate the Boundless SDK builder for estimated and fallback requests

**Files:**

- Modify: `crates/prover/src/boundless/mod.rs`

**Interfaces:**

```rust
async fn build_request(
    &self,
    client: &BoundlessClient,
    quote_context: &QuoteContext,
    /* existing request fields */
) -> RaikoResult<ProofRequest>;
```

When `quote_context.isolated_builder` is true:

```rust
let mut builder = client
    .request_builder
    .as_ref()
    .ok_or_else(|| RaikoError::InvalidRequestConfig(
        "Boundless request builder is not configured".to_string(),
    ))?
    .clone();
builder.preflight_layer = Default::default();
builder.skip_preflight = Some(true);
builder.build(request_params).await
```

`Client::request_builder` is public and optional in boundless-market 2.0.0. Return a configuration error when it is absent; do not reimplement SDK layers. The existing `client.build_request` path remains for Evaluated and Fixed.

**Step 1: Write failing builder-isolation tests**

Using the SDK layer/test doubles available in boundless-market 2.0.0, establish:

- the estimated clone has a distinct default preflight executor cache and no downloader;
- the original shared builder retains its downloader/cache and `skip_preflight` state;
- building with explicit URL input/cycles/journal/image ID through the isolated builder performs no download, local execution, shared-cache read, shared-cache write, or pricing preflight;
- the fallback path also uses isolation after its one local execution;
- Evaluated and Fixed use the shared builder and do not set `skip_preflight`.

Prefer a small private helper returning the selected builder policy so most assertions are pure. Add one async integration test with counting downloader/executor-visible cache behavior for the full SDK guarantee.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::tests::isolated_request_builder_
```

Expected: FAIL while `client.build_request` is unconditional.

**Step 2: Implement direct isolated build with bounded retry**

Retain URL redaction and `retry_external`. Update the stale `ensure_input_uploaded` comment to explain that explicit metadata still triggers SDK best-effort cache fill on the shared builder, hence Estimated/fallback replacement isolation.

**Step 3: Re-run focused tests**

Run the command from Step 1.

Expected: PASS.

**Step 4: Commit**

```bash
git add crates/prover/src/boundless/mod.rs
git commit -m "fix(boundless): isolate estimated request preflight"
```

### Task 9: Preserve quote provenance across resume and same-ID rebids

**Files:**

- Modify: `crates/prover/src/lib.rs`
- Modify: `crates/prover/src/boundless/mod.rs`
- Modify: checkpoint conversion code under `crates/runtime/src/` if its exhaustive field copy requires it

**Step 1: Write failing serialization/rebid tests**

Cover:

- new progress serializes quoted/evaluated counts, strategy, and model ID;
- checkpoint-to-resume copies all quote fields;
- old JSON without added fields still deserializes;
- a same-ID resume/rebid uses the stored quote/journal provenance even when current config changes from Estimated to Fixed or model identity changes;
- a rotated request ID recomputes current quote context;
- legacy stored quoted count but no strategy/model retains that exact count and reports unavailable provenance;
- legacy missing quoted count may poll, but a same-ID rebid fails closed before request construction; after terminal rotation it may prepare from current config.

Run:

```bash
cargo test -p raiko2-prover --features boundless boundless::tests::resume_quote_
cargo test -p raiko2-runtime boundless_submission
```

Expected: FAIL because resume currently drops quote fields and same-ID rebids use current local metadata.

**Step 2: Extend backward-compatible payloads**

All added resume fields use `#[serde(default, skip_serializing_if = "Option::is_none")]`. Keep request digest, image, deadlines, exact max price, and submission attempt validation unchanged.

**Step 3: Make request-ID lineage own quote context**

Store the resume quote context separately from `Submission`. Polling uses it for telemetry; `RebidRequestReuse` carries it only when reusing the ID. Rotation clears it. The journal required to build a rebid is deterministic from the current identical stage input, but the quoted count/strategy/model ID must remain persisted; reject a same-ID rebid if the legacy context lacks its quoted count.

**Step 4: Re-run focused tests**

Run the commands from Step 1.

Expected: PASS.

**Step 5: Commit**

```bash
git add crates/prover/src/lib.rs crates/prover/src/boundless/mod.rs crates/runtime/src
git commit -m "fix(boundless): persist rebid quote provenance"
```

### Task 10: Update operator/API documentation

**Files:**

- Modify: `docs/API.md`
- Modify: `docs/operations.md`
- Modify: `config.example.toml`
- Modify: `docs/superpowers/specs/2026-08-28-boundless-estimated-quote-design.md` only if implementation names differ without changing approved semantics

**Step 1: Update the canonical documentation**

Document exactly three strategies, both required explicit stage tables, per-pair inheritance, Estimated's no-local-execution fast path, warning-plus-local fallback, the empirical 10% publication gate, current proposal operating policy, current aggregation calibrated set from Task 7, release-owner compatibility responsibility, optional progress/resume fields, and intentional rejection of `raiko_agent`.

State explicitly that Estimated does not expose `skip_preflight`, does not submit a proof during estimation, and does not runtime-check ELF/image/revision/RISC0 version.

**Step 2: Verify docs against source**

```bash
rg -n "raiko_agent|estimated|batch_quote|aggregation_quote|skip_preflight" config.example.toml docs/API.md docs/operations.md crates/prover/src bin/raiko2/src
git diff --check
```

Expected: `raiko_agent` appears only in an intentional migration/rejection note and rejection tests; supported-strategy docs and example config match the Rust enum.

**Step 3: Commit**

```bash
git add docs/API.md docs/operations.md config.example.toml docs/superpowers/specs/2026-08-28-boundless-estimated-quote-design.md
git commit -m "docs(boundless): document estimated quote strategy"
```

### Task 11: Complete integration verification and independent review

**Files:**

- Modify only files required by confirmed findings.

**Step 1: Run formatting and focused suites**

```bash
cargo fmt --all -- --check
cargo test -p raiko2-prover --features boundless
cargo test -p raiko2-runtime boundless_submission
cargo test -p raiko2 config::prover
/path/to/venv/bin/python -m pytest experiments/risc0-zkgas/tests -q
git diff --check
```

Expected: PASS.

**Step 2: Run cross-crate static and CI-lane checks**

```bash
cargo clippy --workspace -- -D warnings
cargo test -p raiko2-primitives -p raiko2-primitives-shasta -p raiko2-protocol -p raiko2-protocol-shasta
cargo test -p raiko2-provider -p raiko2-pipeline -p preflight
cargo test -p raiko2-queue -p raiko2-runtime
```

Expected: PASS. No guest rebuild is required because guest source and ELF artifacts are unchanged; the current aggregation ELF is only read for calibration.

**Step 3: Inspect the complete diff and three principal failure modes**

Check the complete branch diff against its base and specifically investigate:

1. an Estimated request accidentally reaches the SDK shared preflight cache or executes twice;
2. a same-ID rebid changes quoted cycles/provenance after restart;
3. a host-derived journal differs from the proposal or aggregation guest journal or hashes malformed carry data through a panic.

**Step 4: Request independent adversarial review**

Give the reviewer the original approved spec and complete diff. Require review of structural/fallback error classification, SDK builder ownership, model single-source enforcement, config migration, journal equivalence, resume compatibility, same-ID lineage, and missing tests. Fix or rebut every material finding with code evidence, then have the same reviewer re-check.

**Step 5: Request independent behavioral verification**

Have a tester independently run the focused model/config/prover/runtime suites and exercise available proposal estimation, operating-policy one-execution fallback, calibrated/unconfigured aggregation counts, serialization compatibility, and builder isolation. Fix confirmed failures and ask the tester to rerun affected checks.

**Step 6: Final readiness check**

```bash
git status --short
git log --oneline --decorate -12
git diff HEAD^ --check
```

Expected: only intentional files are changed, no path contains a user-specific directory, all current checks have recorded results, and independent review/test findings are closed before any PR is opened.
