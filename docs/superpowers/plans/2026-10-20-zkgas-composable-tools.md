# zkGas Composable Tools Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add source-agnostic `zkgas-quote`, `zkgas-compare`, and `zkgas-report` binaries under `bin/zkgas/`, then validate their composition with a real local SP1 execute run without coupling SP1 acquisition to the tools.

**Architecture:** One workspace package exposes strict, independently versioned contracts plus shared identity, arithmetic, and immutable-output helpers. Each binary consumes files and emits files; pricing remains exclusively in `raiko2-zkgas-trace`, while SP1/RISC0 execution and data acquisition remain external producers. Unit and integration tests cover deterministic contracts, joins, metrics, and failure behavior; a bounded manual smoke uses the packaged SP1 ELF and the checked-in proposal fixture.

**Tech Stack:** Rust 1.95 / edition 2024, `clap`, `serde`/`serde_json`, `toml`, `bincode`, `sha2`, `num-bigint`/`num-rational`, Linux `rustix::fs::openat2`, `raiko2-primitives-shasta`, `raiko2-zkgas-trace`, and the existing `guest-launcher` SP1 execution path.

**Spec:** `docs/superpowers/specs/2026-10-20-zkgas-shadow-cli-design.md`

## Global Constraints

- Keep pricing in `raiko2-zkgas-trace::quote_shasta_proposal`; do not copy pricing coefficients or formulas into `bin/zkgas`.
- Do not fetch from Succinct, RISC0, RPC, explorers, or clusters, and do not run a guest from any `zkgas-*` binary.
- Keep `guest-launcher proposal-workload-quote` compatible behind an explicit feature; removing it
  is outside this change, and default launcher builds must not enable MPT instrumentation.
- Every cross-tool contract has its own `schema_version`, uses `#[serde(deny_unknown_fields)]`, and permits arbitrary producer fields only inside size/depth-bounded `metadata` or `source` objects.
- Hash raw input/config files where the spec requires byte identity; hash generated descriptors only after deterministic serialization of structs and `BTreeMap` values.
- Quote identity always refers to canonical bincode, including when `--input-json` is used as a developer convenience.
- Default quote concurrency is exactly one and each `GuestInput` is dropped before the next is decoded; do not load a corpus into memory.
- Existing outputs are reusable only when byte-identical; never overwrite a conflicting file or immutable comparison run.
- Classifications use exact integers/rationals. Six-decimal display ratios round half to even. Unzen conversion is exactly `ceil(proposal_sum * numerator / denominator)` with arbitrary-precision checked bounds at the contract edge.
- `quote.prove.total_pgu` is the only complete new-model SP1-PGU prediction; `node` is informational and raw Unzen is not SP1 PGU.
- Same-unit but incompatible-program ratios are diagnostic only; different-unit rows have no raw ratio.
- Do not build or replace guest ELF/VK artifacts for this work. The SP1 smoke uses the checked-in packaged artifacts.
- Do not commit downloaded corpora, `/tmp` smoke output, or generated campaign reports.

## Review Focus

- Manifest paths changed into symlinks or special files between validation and open must be rejected, not followed; Task 2 adds the race-resistant open and tests.
- Oversized manifest files and aggregate declared input bytes must fail before input decode/replay, while excessive parsed row counts fail before opening any corpus input; Tasks 1 and 2 add boundary tests.
- Duplicate quote/observation keys, including byte-identical duplicates across repeated files, must be structural failures with no comparison store; Task 3 adds cross-file tests.
- Concurrent writers or pre-existing partial/conflicting outputs must never yield a mixed quote, comparison run, or report; Tasks 1, 3, and 4 add idempotence/conflict tests.
- Ratios and normalization near `u128::MAX`, plus exact half-even ties, must not overflow or change classification; Tasks 3 and 4 add arbitrary-precision boundary tests.

---

### Task 1: Package, Strict Contracts, Identity, and Immutable Output

**Files:**
- Modify: `Cargo.toml`
- Modify: `Cargo.lock`
- Create: `bin/zkgas/Cargo.toml`
- Create: `bin/zkgas/src/lib.rs`
- Create: `bin/zkgas/src/contracts.rs`
- Create: `bin/zkgas/src/identity.rs`
- Create: `bin/zkgas/src/output.rs`
- Create: `bin/zkgas/tests/contracts.rs`
- Create: `bin/zkgas/tests/output.rs`

**Interfaces:**
- Consumes: `raiko2_zkgas_trace::ProposalWorkloadQuote` and standard serde-compatible identity fields.
- Produces: `InputManifestV1`, `ObservationManifestV1`, `QuoteRowV1` with `CompleteQuoteV1`/`FailedQuoteV1`, `CompleteObservationV1`/`FailedObservationV1`, `CompatibilityProfilesV1`, `ComparisonManifestV1`, `ComparisonRowV1`, `ReportConfigV1`, `ReportV1`, `QuoteKey`, `ObservationKey`, `ProgramIdentity`, `Workload`, `sha256_hex(&[u8]) -> String`, `canonical_json<T: Serialize>(&T) -> Result<Vec<u8>>`, `content_id(domain: &str, value: &impl Serialize) -> Result<String>`, `executable_identity() -> Result<ExecutableIdentity>`, `commit_file(path: &Path, bytes: &[u8]) -> Result<CommitOutcome>`, and `commit_directory(staging: &Path, final_path: &Path) -> Result<CommitOutcome>`.

- [ ] **Step 1: Add failing strict-contract tests**

  In `bin/zkgas/tests/contracts.rs`, add named tests that assert all eight contracts parse at `schema_version = 1`, reject a wrong version and unknown fields independently, retain sorted bounded `metadata`/`source`, and reject opaque values above constants `MAX_OPAQUE_BYTES = 65_536`, `MAX_OPAQUE_DEPTH = 16`, and `MAX_OPAQUE_ENTRIES = 1_024`. Add an input manifest with more than `MAX_INPUT_ROWS = 10_000` parsed rows and assert it is rejected before any referenced input is opened.

- [ ] **Step 2: Run the contract test and verify it fails**

  Run: `cargo test -p zkgas --test contracts`

  Expected: FAIL because the package and contract types do not exist.

- [ ] **Step 3: Add the workspace package and focused modules**

  Add `bin/zkgas` to workspace members. Use direct package dependencies for `toml = "0.8"`, `num-bigint = "0.4.6"`, `num-rational = "0.4.2"`, and `num-traits = "0.2"`; use existing workspace dependencies for the rest. Define all public wire types in `contracts.rs` with string-encoded `u128` fields where JSON consumers could lose integer precision. Model quote and observation rows as tagged `complete`/`failed` enums so producer failures remain parseable rows; malformed files remain structural errors.

- [ ] **Step 4: Implement deterministic identity helpers and test domain separation**

  Add tests showing reordered `BTreeMap` input hashes identically, while the same payload under `quote-row-v1`, `joined-row-v1`, and `unmatched-row-v1` produces distinct IDs. `executable_identity()` must hash the bytes of `std::env::current_exe()` and include package version, optional source revision, and optional dirty marker only as readable provenance.

- [ ] **Step 5: Add failing immutable-output tests**

  In `bin/zkgas/tests/output.rs`, test new-file commit, byte-identical reuse, conflicting existing file rejection, directory commit, full byte validation of an existing directory, and rejection of a pre-existing partial directory. Include two concurrent writers for identical bytes and two for conflicting bytes; assert one canonical result and no leftover staging paths.

- [ ] **Step 6: Implement atomic file and directory commits**

  Write temporary files/directories beside the destination using `create_new`, sync written files, rename only complete staging content, and validate every expected relative path and byte before reusing an existing comparison directory. Reject symlinks while validating an existing directory. Do not delete conflicts automatically.

- [ ] **Step 7: Run focused tests and commit**

  Run: `cargo test -p zkgas --test contracts --test output`

  Expected: PASS.

  ```bash
  git add Cargo.toml Cargo.lock bin/zkgas/Cargo.toml bin/zkgas/src bin/zkgas/tests/contracts.rs bin/zkgas/tests/output.rs
  git commit -m "feat(zkgas): add strict composition contracts"
  ```

### Task 2: Deterministic Proposal Quoting

**Files:**
- Modify: `bin/zkgas/Cargo.toml`
- Modify: `Cargo.lock`
- Create: `bin/zkgas/src/quote.rs`
- Create: `bin/zkgas/src/bin/zkgas-quote.rs`
- Create: `bin/zkgas/tests/quote.rs`
- Test: `crates/zkgas-trace/src/reconstruct.rs`
- Test: `crates/zkgas-trace/src/runtime_quote.rs`

**Interfaces:**
- Consumes: Task 1 contract/identity/output types and `quote_shasta_proposal(&GuestInput) -> Result<ProposalWorkloadQuote, RuntimeQuoteError>`.
- Produces: `QuoteArgs`, `QuoteLimits`, `run_quote(args: &QuoteArgs, executable: &ExecutableIdentity) -> Result<QuoteRun>`, canonical quote JSONL, and stable exit mapping `complete -> 0`, `committed partial -> 2`, `no structurally valid output -> 1`.

- [ ] **Step 1: Add failing single-input equivalence tests**

  In `bin/zkgas/tests/quote.rs`, serialize the checked-in `tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json` to canonical bincode. Assert `run_quote` emits one complete row whose embedded `ProposalWorkloadQuote` equals a direct `quote_shasta_proposal` call, whose quote key uses the canonical bincode SHA-256/length, and whose single-input descriptor excludes the filesystem path. Add `--input-json` equivalence and assert it produces the same GuestInput identity.

- [ ] **Step 2: Run the quote test and verify it fails**

  Run: `cargo test -p zkgas --test quote single_input`

  Expected: FAIL because `run_quote` and the binary do not exist.

- [ ] **Step 3: Implement single-input parsing and envelope extraction**

  `QuoteArgs` accepts exactly one of `--input-bincode`, `--input-json`, and `--manifest`; single mode requires `--corpus-id` and `--sample-id`. Bincode must round-trip byte-identically. JSON is parsed into the current `GuestInput` without hidden proof-carry backfill, then canonicalized to bincode for identity. Extract and validate manifest/proof-carry/witness chain and proposal IDs, witness chain-spec hashes, ordered active-fork segments, block range/count, public output, model ID, and source descriptor before writing the complete row.

- [ ] **Step 4: Add failing manifest and path-safety tests**

  Add a two-row manifest test for deterministic ordinal order and raw-manifest SHA-256. Add failures for duplicate sample IDs, incorrect hash/length, absolute path, `..`, symlink in any component, FIFO/non-regular input, a symlink-swap attempt, empty proposal, non-canonical bincode, chain/proposal mismatch, per-file byte limit, total byte limit, row-count limit, and elapsed-time budget. Assert every semantically bad input produces a typed failed quote row and exit `2`, while an invalid manifest produces no output and exit `1`.

- [ ] **Step 5: Implement sequential bounded batch quoting**

  Add direct dependency `rustix = { version = "1.1", features = ["fs"] }` and implement `open_manifest_input(manifest_dir: &File, relative: &Path) -> Result<File>` with `openat2`, `OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW`, and `ResolveFlags::BENEATH | ResolveFlags::NO_SYMLINKS | ResolveFlags::NO_MAGICLINKS`; verify the opened descriptor is a regular file. Preflight manifest bytes, parsed row count, declared per-file lengths, and aggregate declared bytes against limits, but stream one input at a time. Preserve raw manifest hash and ordinal, drop input/quote scratch state before the next row, check the elapsed budget before and after every replay, and emit JSONL only after all ordered rows have been serialized.

- [ ] **Step 6: Add CLI/exit/idempotence tests**

  Invoke `env!("CARGO_BIN_EXE_zkgas-quote")` for single and batch cases. Assert exact exit classes, byte-identical output reuse, conflict rejection, and distinct quote identity when only raw manifest bytes change. Add a help-text assertion for the three exclusive input modes and resource-limit defaults.

- [ ] **Step 7: Verify the existing quote core regressions**

  Run: `cargo test -p raiko2-zkgas-trace reconciliation_includes_inline_eip7702_delegation_code`

  Expected: PASS, proving multi-transaction EIP-7702 inline delegation code is still included.

  Run: `cargo test -p raiko2-zkgas-trace checked_in_current_guest_input_completes_quote_with_public_output_parity`

  Expected: PASS.

- [ ] **Step 8: Run quote tests and commit**

  Run: `cargo test -p zkgas --test quote`

  Expected: PASS.

  ```bash
  git add bin/zkgas/src/quote.rs bin/zkgas/src/bin/zkgas-quote.rs bin/zkgas/tests/quote.rs Cargo.toml Cargo.lock
  git commit -m "feat(zkgas): add deterministic proposal quoting"
  ```

### Task 3: Profile Resolution and Immutable Comparison Runs

**Files:**
- Create: `bin/zkgas/src/arithmetic.rs`
- Create: `bin/zkgas/src/compare.rs`
- Create: `bin/zkgas/src/bin/zkgas-compare.rs`
- Create: `bin/zkgas/profiles/sp1-pricing-calibration-v1.toml`
- Create: `bin/zkgas/profiles/artifacts/unzen-sp1-block-comparison-v1.json`
- Create: `bin/zkgas/tests/arithmetic.rs`
- Create: `bin/zkgas/tests/compare.rs`

**Interfaces:**
- Consumes: Task 1 schemas/helpers plus Task 2 quote JSONL.
- Produces: `resolve_profile(quote: &CompleteQuoteV1, observation: &CompleteObservationV1, profiles: &CompatibilityProfilesV1) -> ProfileResolution`, `ceil_mul_ratio(value: &BigUint, numerator: &BigUint, denominator: &BigUint) -> Result<BigUint>`, `format_ratio_half_even(numerator: &BigUint, denominator: &BigUint, places: u32) -> Result<String>`, `run_compare(args: &CompareArgs, executable: &ExecutableIdentity) -> Result<CompareRun>`, and immutable `runs/<run-id>/{manifest.json,rows/<row-id>.json}` stores.

- [ ] **Step 1: Add failing exact-arithmetic tests**

  Cover exact Unzen proposal-sum ceiling, a case that differs from summing per-block ceilings, zero numerator, zero denominator rejection, values above `u128::MAX`, exact six-place half-even ties in both directions, zero prediction, and positive/zero actual classification. Assert classification is made from integers before formatting.

- [ ] **Step 2: Run the arithmetic test and verify it fails**

  Run: `cargo test -p zkgas --test arithmetic`

  Expected: FAIL because the arithmetic module does not exist.

- [ ] **Step 3: Implement arbitrary-precision arithmetic and sealed profiles**

  Import the original sealed Unzen normalization bytes unchanged as `bin/zkgas/profiles/artifacts/unzen-sp1-block-comparison-v1.json`; verify raw file SHA-256 `3856ea6deca594d80c8d68d698cfce1faac037110f02049a6db2c0c6a47b72c7` and preserve its internal artifact identity `9d97015d76b8c5d66f28f8d220138f91aee740adcd083d62f5938219b5a5470e` separately. In the wrapper profile, encode the exact lossless rational interpretation of `kappa_unzen` as numerator `210937703648831249547430320136465120349542602443640317268060183142176006857255572450221059359429167` over denominator `2500000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000`, plus source/target units, diagnostic-only status, and source program identity. The embedded SP1 pricing profile must bind model ID `93d04eddda3779393807808859eb36561d35aad0cdd8a8a3960ec3e279a93046`, calibration proposal ELF SHA-256 `ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7`, and calibration serialized-VK artifact SHA-256 `bd71c1f1cbd72387aa057ae6e936afd6dcb0bc170c6a91193c6d1a0b47507102` as separately named algorithms.

- [ ] **Step 4: Add failing join/profile tests**

  Cover one quote joined to repeated SP1 attempts, unmatched quote and observation rows, parseable upstream failed quote/observation rows, identical and conflicting duplicate keys within and across repeatable files, zero/one/multiple matching profiles, digest-algorithm mismatch, same-unit incompatible program, different-unit RISC0 observation, and a custom compatible SP1 profile. Assert no profile match emits exactly one `incompatible_profile` row, overlapping profiles emit exactly one `ambiguous_profile` row and partial status, and neither case expands rows per candidate profile.

- [ ] **Step 5: Implement joins, metric eligibility, and tagged row IDs**

  Load all input files only after per-file and aggregate byte/count bounds pass. Reject any repeated quote or observation primary key structurally with no run. Build collision-separated IDs for `joined`, `incompatible_profile`, `ambiguous_profile`, `unmatched_quote`, `unmatched_observation`, `invalid_quote`, and `invalid_observation`. Compatible SP1 rows expose exact ratios and underquote classification; same-unit incompatible rows expose only a `cross_program_ratio` diagnostic; different-unit rows omit raw ratios. Apply optional normalization only when every named source/target identity matches.

- [ ] **Step 6: Add immutable-store and exit tests**

  Assert manifest identity binds ordered input hashes, executable SHA-256, profile/config hashes, arithmetic policy, all ordered `(comparison_row_id,row_sha256)` pairs, and the sequence hash. Test complete exit `0`, upstream/unmatched/ambiguous partial exit `2`, incompatible-only exit `0`, byte-identical run reuse, tampered existing row rejection, and conflicting existing run rejection.

- [ ] **Step 7: Run compare tests and commit**

  Run: `cargo test -p zkgas --test arithmetic --test compare`

  Expected: PASS.

  ```bash
  git add bin/zkgas/src/arithmetic.rs bin/zkgas/src/compare.rs bin/zkgas/src/bin/zkgas-compare.rs bin/zkgas/profiles bin/zkgas/tests/arithmetic.rs bin/zkgas/tests/compare.rs Cargo.toml Cargo.lock
  git commit -m "feat(zkgas): compare quotes with workload observations"
  ```

### Task 4: Integrity-Checked Aggregate Reports

**Files:**
- Create: `bin/zkgas/src/report.rs`
- Create: `bin/zkgas/src/bin/zkgas-report.rs`
- Create: `bin/zkgas/tests/report.rs`

**Interfaces:**
- Consumes: Task 3 immutable comparison run and Task 1 `ReportConfigV1`.
- Produces: `verify_comparison_run(path: &Path) -> Result<VerifiedComparisonRun>`, `build_report(run: &VerifiedComparisonRun, config: &ReportConfigV1, executable: &ExecutableIdentity) -> Result<ReportV1>`, `run_report(args: &ReportArgs, executable: &ExecutableIdentity) -> Result<ReportRun>`, and deterministic report JSON.

- [ ] **Step 1: Add failing integrity tests**

  Build a tiny comparison store and assert missing, extra, renamed, reordered, symlinked, or byte-modified row files fail before filtering or aggregation. Assert a changed manifest, row-sequence hash, report config, or report executable identity changes the report ID. Add identical-output reuse and conflicting-output rejection.

- [ ] **Step 2: Run the report test and verify it fails**

  Run: `cargo test -p zkgas --test report integrity`

  Expected: FAIL because reporting is not implemented.

- [ ] **Step 3: Implement full-run verification**

  Parse the manifest strictly, enumerate only regular `rows/*.json` files without following symlinks, require the exact declared filename set, verify each row hash and row ID, and recompute the ordered sequence hash before applying `ReportConfigV1`.

- [ ] **Step 4: Add failing aggregation tests**

  Cover homogeneous grouping by chain ID plus ordered fork topology and relative transition offsets, backend/unit/stage, complete program identity/backend version, model/profile, and normalization identity. Absolute block heights for ordinary same-fork proposals must aggregate, while single-fork rows cannot merge with a cross-fork proposal. Cover count-weighted and workload-weighted ratios, underquote rate, zero-actual accounting, nearest-rank quantiles, Pearson/Spearman/Kendall and predeclared bucket amplification for incompatible-unit series, exclusions with reasons, and no compliance/capacity/admission claim fields.

- [ ] **Step 5: Implement deterministic metrics and filtering**

  Compute exact aggregate sums/rates with big integers/rationals, use stable tie ranking for Spearman and deterministic pair counting for Kendall, and emit finite decimal diagnostics with the method named in the report. Record whole-run counts before filters and selected counts after filters. Never aggregate heterogeneous strata merely because the config omitted a filter.

- [ ] **Step 6: Add exit-propagation tests**

  Assert a complete comparison run reports with exit `0`; a partial run reports with exit `2` even when filters exclude every partial row; incompatible/unavailable metrics do not make a complete run partial; structural integrity failure exits `1` and commits no report.

- [ ] **Step 7: Run report tests and commit**

  Run: `cargo test -p zkgas --test report`

  Expected: PASS.

  ```bash
  git add bin/zkgas/src/report.rs bin/zkgas/src/bin/zkgas-report.rs bin/zkgas/tests/report.rs
  git commit -m "feat(zkgas): add integrity-checked reports"
  ```

### Task 5: Operator Documentation and Deterministic End-to-End Coverage

**Files:**
- Create: `bin/zkgas/README.md`
- Create: `bin/zkgas/tests/end_to_end.rs`
- Modify: `README.md`

**Interfaces:**
- Consumes: all three binaries and all eight versioned contracts.
- Produces: the supported operator workflow and a deterministic quote → compare → report regression using checked-in input plus generated temporary observations.

- [ ] **Step 1: Add a failing end-to-end test**

  Generate canonical bincode from the checked-in mainnet proposal fixture, invoke `zkgas-quote`, generate one compatible synthetic SP1 observation and one incompatible-unit RISC0 observation, invoke `zkgas-compare`, then invoke `zkgas-report`. Assert quote public-output parity, joined row counts, metric eligibility, immutable hashes, homogeneous strata, and exit `0`. Add a second run with an unmatched observation and assert compare/report exit `2` while committed accounting stays readable.

- [ ] **Step 2: Run the end-to-end test and verify any missing behavior fails**

  Run: `cargo test -p zkgas --test end_to_end`

  Expected: FAIL until all binary argument/help/output details are aligned.

- [ ] **Step 3: Write `bin/zkgas/README.md` from actual `--help` output**

  Document purpose and non-goals; the three commands; the eight-contract table with producer, consumer, schema version, unit, and compatibility rule; all inputs/outputs; exact exit classes; resource limits; immutable-output behavior; and examples for single JSON/bincode quote, batch quote, compatible SP1 comparison, cross-program SP1 diagnostic, RISC0 comparison, and re-reporting without replay. State that acquisition and product admission remain out of scope.

- [ ] **Step 4: Link the tool suite from the repository README**

  Add one concise developer-tool entry under the existing guest/profiling documentation. Do not add a server config or deployment narrative.

- [ ] **Step 5: Verify docs and commit**

  Run: `cargo run -q -p zkgas --bin zkgas-quote -- --help`

  Run: `cargo run -q -p zkgas --bin zkgas-compare -- --help`

  Run: `cargo run -q -p zkgas --bin zkgas-report -- --help`

  Expected: all commands exit `0`, and names/defaults match `bin/zkgas/README.md`.

  Run: `cargo test -p zkgas --test end_to_end`

  Expected: PASS.

  ```bash
  git add bin/zkgas/README.md bin/zkgas/tests/end_to_end.rs README.md
  git commit -m "docs(zkgas): document composable evaluation workflow"
  ```

### Task 6: Real Packaged-SP1 Composition Smoke

**Files:**
- No repository files are created or modified.
- Runtime output: a fresh directory under `/tmp`, removed or retained only for the current diagnostic session and never committed.

**Interfaces:**
- Consumes: checked-in GuestInput JSON, packaged `sp1_shasta_proposal.elf`/VK, `guest-launcher`, and the three completed `zkgas-*` binaries.
- Produces: one external observation manifest from real `ExecutionReport::gas()`, one immutable comparison run, one report, and captured command/resource evidence.

- [ ] **Step 1: Build host binaries without rebuilding guests**

  Run: `cargo build -r -p zkgas -p guest-launcher`

  Expected: PASS; `crates/guests/elf/` remains unchanged.

- [ ] **Step 2: Quote the exact checked-in proposal input**

  Create a fresh `/tmp/raiko2-zkgas-sp1-smoke-*` directory with `mktemp -d`, then run `target/release/zkgas-quote --input-json tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json --corpus-id raiko2-checked-in --sample-id mainnet-proposal-23077 --output <tmp>/quotes.jsonl`.

  Expected: exit `0`, one complete row, nonzero `quote.prove.total_pgu`, and quote public output matching native quote tests.

- [ ] **Step 3: Execute the same proposal with packaged SP1**

  Run with a 30-minute bound and resource telemetry:

  ```bash
  timeout 30m /usr/bin/time -v target/release/guest-launcher \
    --stage proposal \
    --proof-type sp1 \
    --mode execute \
    --sp1-prover local \
    --input tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json \
    --json-out <tmp>/sp1-execute.json
  ```

  Expected: exit `0`; `gas` and `primary_workload_metric.count` are equal positive SP1 PGU values; public output equals the quote row; peak RSS and elapsed time are recorded. An exit `124` is a timeout, `137` is only SIGKILL evidence unless kernel/cgroup logs independently prove OOM.

- [ ] **Step 4: Emit an external observation manifest**

  First run `cargo run -r -p xtask-build-guest --bin guest-digests --features digests -- --output <tmp>/guest-digests.json` to derive the packaged proposal VK's `vk_bn254` and `vk_hash_bytes` without rebuilding a guest. Then use `~/.venv/bin/python` only as an external producer to read `<tmp>/quotes.jsonl`, `<tmp>/sp1-execute.json`, and that digest summary; compute SHA-256 for `crates/guests/elf/sp1_shasta_proposal.elf` and `sp1_shasta_proposal.vk.bin`; and write strict `ObservationManifestV1` to `<tmp>/sp1-observations.json`. Set `backend = "sp1"`, `workload.unit = "sp1_pgu"`, `measurement_stage = "execute"`, preserve every digest algorithm separately, and preserve the guest-launcher command/report hashes in `source`. Do not add this conversion to a `zkgas-*` binary.

- [ ] **Step 5: Compare and report the real observation**

  Run `target/release/zkgas-compare --quotes <tmp>/quotes.jsonl --observations <tmp>/sp1-observations.json --output <tmp>/comparison-store`, then create a strict report config and run `target/release/zkgas-report --comparisons <tmp>/comparison-store/runs/<run-id> --config <tmp>/report.toml --output <tmp>/report.json`.

  Expected: both tools exit `0`; the row is a same-unit `incompatible_profile` diagnostic because the packaged proposal ELF SHA-256 is `eb65a5ec8750854c56f06ade6abf10dd7eac1c1a122c6c4a5dde867c189a234d`, not the calibration ELF `ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7`. The row may contain a cross-program ratio, but must contain no `underquoted`, model-error, safety, or admission classification.

- [ ] **Step 6: Record the smoke evidence in the PR description only**

  Report the exact commit, executable hashes, GuestInput hash, packaged ELF/VK hashes, predicted PGU, actual SP1 PGU, diagnostic ratio, public-output equality, elapsed time, peak RSS, exit classes, and incompatibility reason. Do not commit the temporary observation, comparison store, or report.

### Task 7: Whole-Branch Verification and Independent Review

**Files:**
- Review: complete diff against the PR base.
- Modify: only files required to fix confirmed findings.

**Interfaces:**
- Consumes: all prior tasks and the approved design.
- Produces: a ready-to-review branch with independent adversarial review and behavioral verification evidence.

- [ ] **Step 1: Run formatting and focused suites**

  Run: `cargo fmt --all -- --check`

  Run: `cargo test -p zkgas`

  Run: `cargo test -p raiko2-zkgas-trace`

  Expected: PASS.

- [ ] **Step 2: Run workspace compatibility checks**

  Run: `cargo clippy --workspace -- -D warnings`

  Run: `cargo test -p raiko2-primitives -p raiko2-primitives-shasta -p raiko2-protocol -p raiko2-protocol-shasta`

  Run: `cargo test -p raiko2-provider -p raiko2-pipeline -p preflight`

  Expected: PASS, or report a pre-existing unrelated failure with exact evidence rather than claiming success.

- [ ] **Step 3: Inspect the complete branch diff and path hygiene**

  Run: `git diff --check <base>...HEAD`

  Run: `git diff --stat <base>...HEAD`

  Inspect every new path, README command, embedded profile/artifact hash, and contract field. Verify no user-specific absolute path, downloaded corpus, generated report, or guest artifact change is present.

- [ ] **Step 4: Request independent adversarial code review**

  Give a fresh reviewer the original user request, approved design, and complete diff. Require review of contract strictness, identity collision domains, path confinement/TOCTOU, exact arithmetic, partial exit propagation, immutable output, and the boundary between external SP1 production and `zkgas-*` tools.

- [ ] **Step 5: Request independent behavioral verification**

  Give a separate tester the original request and built binaries. Require synthetic complete/partial end-to-end runs, tamper tests, duplicate cross-file keys, symlink inputs/rows, and validation of the real SP1 smoke evidence without trusting the implementation author's explanation.

- [ ] **Step 6: Fix or rebut all material findings and re-run affected checks**

  Allow up to two review/fix iterations while material findings continue. The agent that raised a finding re-checks the fix. Commit focused fixes with Conventional Commit messages.

- [ ] **Step 7: Push the branch and update PR #281**

  Push only after all required checks and independent gates complete. Update the PR description with implemented scope, exact verification commands, real SP1 smoke evidence, known cross-program limitation, and the explicit non-promotion boundary.
