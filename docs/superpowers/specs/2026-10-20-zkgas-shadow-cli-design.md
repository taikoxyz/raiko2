# zkGas Composable Tools Design

## Status and Boundary

Approved direction: add a `bin/zkgas/` workspace package containing small composable command-line
tools for computing and evaluating zkGas. It is not a node admission component, prover-runtime
integration, service, or deployment. Promotion of the pricing model into a real product is a
separate future decision made after enough evidence is collected.

The tools are source-agnostic. Any producer may supply a valid current-format `GuestInput`: a public
SP1 task, a local SP1 run, a RISC0 run, a saved fixture, or another source. Succinct is not a trust
boundary or dependency of the zkGas tool suite.

## Goals

Provide composable commands that:

1. compute the canonical new-model quote for any valid proposal `GuestInput`;
2. emit enough proposal, chain, model, and input identity to join that quote safely;
3. compare quotes with separately supplied backend workload observations without mixing units,
   programs, networks, or execution stages; and
4. generate deterministic row-level and aggregate reports for iterative model evaluation.

The comparison framework supports SP1 PGU now and permits RISC0 cycles or future backend metrics
without changing the quote command or coupling it to a collector.

## Non-Goals

- Do not fetch Succinct, RISC0, RPC, explorer, or cluster data inside the tools.
- Do not reconstruct `GuestInput` from proposal or block ranges.
- Do not run an SP1 or RISC0 guest to obtain truth.
- Do not add a raiko2 server endpoint, background job, config flag, or runtime persistence path.
- Do not change headers, difficulty, transaction admission, proof requests, or proving behavior.
- Do not make a production-admission, capacity, or promotion claim from a comparison report.
- Do not support historical `GuestInput` schema translation in the initial version.
- Do not commit downloaded corpora or generated campaign reports to this repository.

## Tool Composition

The workflow consists of independent file-oriented tools:

```text
input producer ──> GuestInput ──> zkgas-quote ──> quote rows ───────────────┐
truth producer ────────────────> workload observations ──> zkgas-compare ──┤
                                                          comparison rows ──> zkgas-report
```

An input producer owns acquisition and retention. A truth producer measures or retrieves backend
workload. For today's daily SP1 campaign, the existing public-task collector may perform both roles.
A future RISC0 collector emits the same observation schema with a different backend, unit, and
program identity.

`zkgas-quote` checks byte identity and internal `GuestInput` consistency. `zkgas-compare` owns join,
unit, and compatibility decisions. `zkgas-report` owns filtering and aggregation. None proves that a
producer measured the claimed input; that relationship is an explicit assertion in the observation
row and remains part of corpus provenance.

No tool invokes another tool internally. Their versioned JSON/JSONL contracts are the composition
boundary, so a caller may replace any producer or consume any intermediate output directly.

## Package Layout

Add one workspace package:

```text
bin/zkgas/
  Cargo.toml
  README.md
  src/lib.rs
  src/bin/zkgas-quote.rs
  src/bin/zkgas-compare.rs
  src/bin/zkgas-report.rs
```

The Cargo package is named `zkgas` and declares the three binary names shown above.
`src/lib.rs` owns only the shared strict schemas, identity helpers, exact arithmetic, and atomic
output helpers. Pricing remains in `raiko2-zkgas-trace`; the tools contain no duplicate pricing
implementation.

`bin/zkgas/README.md` is required. It documents each tool's purpose, inputs, outputs, units, exit
codes, resource limits, and examples for single-input quote, batch quote, SP1 comparison, RISC0
comparison, and regenerating a report from existing comparison rows. It also states that data
acquisition and production admission are out of scope.

## Commands

### `zkgas-quote`

```text
zkgas-quote \
  --input-bincode ./guest-input.bin \
  --corpus-id ad-hoc-check \
  --sample-id mainnet-proposal-123 \
  --output ./quote.jsonl
```

The same tool supports a corpus manifest:

```text
zkgas-quote \
  --manifest ./corpus/manifest.json \
  --output ./quotes.jsonl
```

Exactly one of `--input-bincode` and `--manifest` is accepted. A single input requires explicit
`--corpus-id` and `--sample-id`; a manifest supplies them. JSON `GuestInput` may be supported as a
developer convenience, but the tool re-encodes it and identities always refer to canonical bincode.
The command calls `raiko2-zkgas-trace::quote_shasta_proposal`; source-specific acquisition logic is
not linked into the package.

Single and batch modes emit the same versioned quote-row JSONL contract, so every downstream tool
accepts either output without conversion.

Each input-manifest row supplies a unique `sample_id`, a manifest-relative bincode path, SHA-256,
and byte length. Paths are normalized and must remain beneath the manifest directory. Symlinks,
non-regular files, absolute paths, and parent traversal are rejected. Rows are processed with
bounded concurrency and deterministic output ordering. Every manifest row produces either a
complete quote row or a typed failure row; failures are never silently dropped.

`zkgas-quote` reads but does not copy, download, delete, or retain input artifacts. Corpus storage
is deliberately owned by the input producer. Reports bind the manifest hash and input hashes so a
caller can determine whether the referenced corpus is still available. A single-quote output and a
batch output are written atomically. If the exact output path exists, the tool reuses it only when its
bytes match the newly computed result; otherwise it fails rather than overwriting it.

### `zkgas-compare`

```text
zkgas-compare \
  --quotes ./quotes.jsonl \
  --observations ./sp1-observations.json \
  --profiles ./profiles.toml \
  --output ./comparison-store
```

`--quotes` and `--observations` are repeatable, allowing independently produced corpora and backend
observations to be joined in one comparison run.

The quote primary key is `(corpus_id, sample_id, guest_input_sha256)`. Each observation references
that key as a foreign key but has its own primary key
`(observation_set_id, observation_id, attempt_id)`, so one quote may join to any number of SP1,
RISC0, or repeated observations. A repeated quote primary key or observation primary key is a
structural error whether its bytes are identical or conflicting; the tool never silently deduplicates
them. Unmatched quote and observation rows remain visible in the output accounting. The joined-row
identity additionally contains the compatibility-profile ID.

The content-addressed profile config declares compatible backend/program profiles and optional
normalization artifacts. An embedded initial SP1 profile is available for the current pricing model;
a file override is explicit and recorded in the comparison manifest. The tool emits immutable
row-level comparison results and accounting; it does not aggregate or filter them.

### `zkgas-report`

```text
zkgas-report \
  --comparisons ./comparison-store/runs/<run-id> \
  --config ./report.toml \
  --output ./report.json
```

The content-addressed report config declares filters, grouping, quantiles, and predeclared workload
buckets. Optional filters such as chain ID, backend, or program ID are recorded in the report.
Without a filter, heterogeneous rows remain visible but are stratified; they are never silently
combined into one compatibility result. Re-reporting never reruns native proposal execution or
changes the row-level compatibility decision.

The existing `guest-launcher proposal-workload-quote` developer interface remains compatible in
this PR behind the explicit `zkgas-quote` Cargo feature. It and `zkgas-quote` use the same quote
implementation, while ordinary launcher builds do not enable MPT instrumentation through feature
unification. Removing that interface is a separate compatibility decision.

## Versioned Composition Contracts

Every cross-tool file has a required initial `schema_version` and rejects unknown fields. The only
extension containers are the explicitly bounded input `metadata` and observation `source` objects.
JSONL repeats its row schema version on every line so a sliced stream remains self-describing. The
initial contracts are independently versioned:

- input manifest;
- observation manifest;
- quote row;
- compatibility-profile config;
- comparison manifest;
- comparison row;
- report config; and
- report output.

Changing one contract does not implicitly change another. `bin/zkgas/README.md` contains a contract
table with the current version, producer, consumer, units, and compatibility rule for every item.

### Input manifest row

The input manifest has a strict `schema_version`, a producer-chosen `corpus_id`, and an ordered
sample list. Unknown top-level or row fields are rejected; arbitrary producer data is allowed only
inside the bounded `metadata` object. An input row contains:

- `sample_id`: producer-assigned stable identifier, unique within the manifest;
- `guest_input.path`: path relative to the manifest;
- `guest_input.sha256` and `guest_input.byte_length`; and
- optional opaque producer metadata that is preserved but not interpreted.

`zkgas-quote` opens each path without following symlinks, verifies it is a regular file still
confined under the manifest directory, and then verifies the declared byte identity before decoding.
It rejects non-canonical bincode, an input that fails proposal validation, an empty proposal,
arithmetic overflow, and an identity mismatch with a typed row result.

### Workload observation row

The observation manifest has a strict `schema_version`, an `observation_set_id`, and an ordered
observation list. Unknown fields are rejected except inside the bounded `source` object. An
observation contains:

- `observation_id`, identifying the measurement series within its observation set;
- an explicit `attempt_id`; one-off measurements may use their request/run ID, while repeated
  measurements use distinct attempt IDs and share the same observation ID or an optional
  `replicate_group`;
- `corpus_id`, `sample_id`, and `guest_input_sha256`;
- `backend`: for example `sp1` or `risc0`;
- `workload.value`: a non-negative integer;
- `workload.unit`: for example `sp1_pgu` or `risc0_cycles`;
- `measurement_stage`: for example `execute`, `prove`, or `estimate`;
- program identity with explicitly named algorithms, including ELF or image SHA-256 and the
  backend-native VK/image identifier where applicable;
- backend and software version when known; and
- an opaque `source` object identifying how the producer obtained the observation.

The tools treat `source` as provenance, not authority. They never contact that source. Program
identifiers with different algorithms are stored separately and are never compared as though they
were the same digest.

## Quote Identity and Validation

Each complete quote row contains the full `ProposalWorkloadQuote` and an evaluation envelope with:

- schema version and required `zkgas-quote` executable SHA-256;
- package version plus optional source revision and dirty marker as readable provenance;
- `corpus_id`, `sample_id`, canonical GuestInput SHA-256, and byte length;
- input-source identity: batch mode records the SHA-256 of the exact raw input-manifest bytes and row
  ordinal, while single mode records a canonical single-input invocation-descriptor SHA-256;
- the bounded producer metadata from the input row, when present;
- pricing `model_id` and output schema identity;
- `taiko.proposal_id`;
- manifest chain name and chain ID;
- proof-carry chain ID and proposal ID;
- the canonical hash of every witness chain spec;
- first and last block number, block count, and public output;
- the ordered active-fork segments across that block range; and
- status and a typed failure when validation does not complete.

The single-input descriptor binds the command schema, corpus ID, sample ID, canonical GuestInput
identity, and applicable limits without embedding machine-specific paths. Comparison rows retain the
complete quote envelope; comparison row hashes and the report's verified row-sequence hash therefore
bind the input manifest or single-input descriptor transitively.

`zkgas-quote` requires the manifest, proof-carry data, and every witness to agree on chain and
proposal identity wherever the current data model carries the same fact. It also verifies contiguous
block topology through the canonical proposal-validation path. A disagreement is a typed failure,
not a separate stratum.

Quote JSONL streams one serialized row at a time into an atomically published sibling staging file.
Its 64 MiB hard output ceiling is the same as compare's per-file input ceiling, so every successfully
published quote file is consumable downstream. Compare treats quote files as external input and
revalidates all duplicated canonical-input, proposal/block/fork/public-output, pricing-model/surface,
and prediction-arithmetic identities before joining or computing metrics. Empty quote or observation
populations are structural collection failures and do not publish a comparison run.

This identity prevents mainnet, Hoodi, devnet, and Masaya samples from being confused even when they
share a requester, program, or block number.

On Linux, executable identity opens `/proc/self/exe` and hashes that pinned running image rather than
resolving and later reopening a replaceable pathname.

## Pricing Surfaces

`raiko2-zkgas-trace::quote_shasta_proposal` remains the only quote implementation. The envelope
exposes two distinct surfaces:

- `node`: incomplete block-construction-visible accounting, including raw Unzen difficulty;
- `prove`: full native proposal replay, including exact block-end resources, MPT, payload, blob, and
  proposal-once work.

Only `quote.prove.total_pgu` is the current model's proposal-level SP1 PGU prediction. Its envelope
labels the series with unit `sp1_pgu`, the prove surface, and the pricing calibration profile. The
`node` surface is informational and must not be compared with proposal truth as a complete
prediction. Proposal observations are never presented as per-block truth.

The quote row also exposes raw Unzen proposal sum with unit `unzen_zkgas`. Raw Unzen is a different
unit from SP1 PGU and RISC0 cycles and is never described directly as overquoting or underquoting
either backend.

## Compatibility Profiles and Units

Every score series and observation has an explicit unit, backend profile, program identity, and
measurement stage.

For each joined quote/observation pair, profile resolution is exactly one of:

- exactly one matching profile: emit a classified joined row using that profile ID;
- no matching profile: emit one incompatible row using the profile-config identity and a
  `no_matching_profile` tag; or
- more than one matching profile: emit one typed `ambiguous_profile` failure containing the ordered
  matching profile IDs and make the comparison run partial.

The initial version never expands one observation into multiple profile rows. Profile definitions
must therefore be disjoint for any quote/observation identity.

Direct prediction-error metrics are enabled only when all of these match a declared compatibility
profile. For the initial model that means:

- prediction is `quote.prove.total_pgu` with unit `sp1_pgu`;
- observation unit and stage are the profile's SP1 PGU truth surface; and
- the observed program identity matches the pricing calibration identity using the same digest
  algorithms.

The pricing calibration identity is provenance-bound to the pinned Alethia Reth
`runtime-spec.json` and `RUNTIME_SPEC_SHA256`. Its production ELF SHA-256, serialized-VK artifact
SHA-256, and any reviewed binding to an SP1 vkey digest remain separately named fields.

For the same unit but an incompatible program, `zkgas-compare` may expose the numerical ratio only as a
cross-program diagnostic and must not classify it as model error, overquote, underquote, or a safety
result. For a different unit, it does not compute a raw ratio. Both cases may report scale-invariant
workload-shape metrics such as raw-value Pearson, Spearman, Kendall, and predeclared bucket
amplification. This permits future RISC0 comparison without pretending RISC0 cycles and SP1 PGU are
the same unit.

An optional immutable normalization profile may map one named score unit and program profile to one
named observation unit and program profile. Normalized results are always labeled diagnostics. The
profile preserves the original calibration artifact bytes and hash; a wrapper records the exact
lossless rational interpretation used by the tools as a separate identity.

For the existing Unzen scalar, the only proposal-level conversion is:

```text
calibrated_unzen = ceil(unzen_proposal_sum * numerator / denominator)
```

It uses checked integer arithmetic and never sums per-block rounded values or parses through binary
floating point.

## Comparison Rows

`--output` names a comparison store root. Each run creates an immutable directory containing:

```text
runs/<run-id>/manifest.json
runs/<run-id>/rows/<comparison-row-id>.json
```

The run ID is the SHA-256 of a canonical descriptor binding the exact quote, observation, and
profile-config file hashes, required `zkgas-compare` executable SHA-256, model/profile identities,
arithmetic policy, and ordered tagged comparison-row IDs. Package version and source revision do not
substitute for the executable content identity. Output filenames are the SHA-256 comparison-row IDs,
never producer-controlled IDs. Files are written through a staging directory and committed
atomically. Reusing an existing run ID byte-validates every file and never silently overwrites a
conflict.

Every comparison row has a tagged `comparison_row_id` with a collision-separated canonical input:

- `joined`: quote primary key, observation primary key, and resolved profile ID;
- `incompatible_profile`: quote primary key, observation primary key, profile-config hash, and the
  `no_matching_profile` tag;
- `unmatched_quote`: quote primary key and quote-row content hash;
- `unmatched_observation`: observation primary key and observation-row content hash;
- `invalid_quote`: available quote identity fields and quote-row content hash; or
- `invalid_observation`: available observation identity fields and observation-row content hash.

An `ambiguous_profile` row uses the `joined` identity domain plus the ordered matching profile IDs.
The comparison manifest records every ordered `(comparison_row_id, row_sha256)` pair and a hash of
that full sequence, a `complete` or `partial` status, and counts by row category. Before reporting,
`zkgas-report` verifies the manifest, every row hash, and the sequence hash. Missing, extra,
reordered, or modified rows are structural failures.

Every joined row records the full quote envelope, observation, compatibility decision, and applicable
metrics. Unmatched, invalid, incompatible, and failed rows are retained with typed reasons.

For a compatible positive observation `actual` and prediction `prediction`, compute with exact
rational arithmetic:

```text
prediction_over_actual = prediction / actual
actual_over_prediction = actual / prediction
underquoted = prediction < actual
```

A positive actual with zero prediction is an explicit infinite underpricing case. A zero actual is
classified separately. Display ratios use six fractional digits and round half to even;
classifications use unrounded integers.

## Aggregate Report

`zkgas-report` consumes one immutable comparison run. Its output contains a report ID derived from
the verified comparison manifest hash, bound ordered-row-sequence hash, report-config hash, required
`zkgas-report` executable SHA-256, arithmetic policy, filters, and grouping rules. The report is
written atomically; an existing output is reused only when it is byte-identical.

For every homogeneous stratum, the report states the comparison-eligible population and computes:

```text
count_weighted_prediction_over_actual = sum_i(prediction_i / actual_i) / row_count
workload_weighted_prediction_over_actual = sum_i(prediction_i) / sum_i(actual_i)
underquote_rate = count_i(prediction_i < actual_i) / row_count
```

Zero-actual rows are counted separately and excluded from ratio quantiles. Quantiles use nearest rank
with the convention and population recorded. Strata include at least:

- chain ID and ordered active-fork topology, with every transition represented by its block offset
  relative to the proposal start (not by an absolute block number);
- backend, workload unit, and measurement stage;
- program identity and backend version;
- pricing model ID and compatibility profile; and
- normalization profile and compatibility, when used.

The report also includes scale-valid workload-recognition metrics for incompatible-unit series and
lists every excluded or unavailable metric with its reason. It must not claim evaluation-standard
compliance, capacity safety, or admission readiness merely because an observed corpus has no
underquote.

## Failure and Resource Policy

Failures include manifest/schema errors, duplicate join keys, missing files, byte-identity mismatch,
GuestInput decode or canonicalization failure, proposal validation failure, chain/proposal mismatch,
quote overflow, invalid normalization, and output conflict. A valid incompatible unit or profile is
a classified result, not a processing failure.

Batch input count, individual input bytes, total bytes read, concurrency, and processing time have
explicit limits. Default concurrency is one because native replay is memory-intensive. Each input is
released before the next begins; the command never loads the full corpus into memory.

A row-level failure does not erase other results, but a structurally invalid manifest, ambiguous
join, or inability to commit the requested output returns nonzero. Partial results remain explicitly
marked.

Exit classes are stable for automation:

- `zkgas-quote` returns `0` only when every requested row completes and `2` when it commits a valid
  stream containing one or more failed rows, including when every row failed;
- `zkgas-compare` returns `0` only when all input rows are valid, every expected join is resolved, and
  no ambiguous profile exists; it returns `2` after committing any upstream failure, unmatched row,
  invalid row, or ambiguous-profile row;
- `zkgas-report` returns `2` whenever the verified comparison manifest is partial, even if its
  filters exclude every partial row; it returns `0` only for a complete source run and a successfully
  committed report; and
- `1`: usage/configuration error or no structurally valid output could be committed.

A correctly classified incompatible profile or unavailable metric is not a processing failure for
any tool. Every report records whole-run partial counts before filters as well as selected-population
counts after filters, so filtering cannot hide or downgrade upstream incompleteness.

## Verification

Implementation requires focused automated coverage for:

- quote equivalence between `zkgas-quote` and the existing guest-launcher entry point;
- `bin/zkgas/README.md` command, schema, unit, composition, limit, and exit-code examples that match
  the implemented `--help` output;
- independent strict versions and unknown-field rejection for all eight composition contracts, plus
  bounded opaque metadata;
- current bincode canonical round-trip and invalid-input failure;
- exact input hash/length checks plus absolute, parent-traversal, symlink, and special-file rejection;
- batch manifest and row-ordinal provenance, canonical single-input descriptors, and a changed
  manifest with identical GuestInput producing distinct quote/report identities;
- proposal ID, chain ID, chain-spec, active-fork, block-range, and public-output extraction;
- cross-fork proposal stratification that cannot merge with either adjacent single-fork stratum;
- mixed mainnet/Hoodi/dev/Masaya fixtures and mismatch rejection;
- deterministic batch ordering, bounded memory, and typed partial rows;
- one-to-many quote/observation joins, repeated-attempt identity, identical and conflicting duplicate
  rejection, and unmatched accounting;
- collision-separated identities for joined, unmatched, invalid, and ambiguous-profile rows;
- comparison-row-ID filenames and run descriptors covering every complete and partial row variant;
- zero, exactly-one, and overlapping-profile resolution fixtures;
- SP1 compatible-unit direct metrics, zero prediction, zero actual, rounding, and quantiles;
- RISC0 or synthetic incompatible-unit rows that permit only scale-valid recognition metrics;
- program/digest algorithm mismatch and calibration incompatibility labeling;
- exact Unzen proposal-sum normalization and a guard against raw Unzen/backend ratios;
- homogeneous stratification and prevention of cross-chain/program/backend aggregation;
- comparison row-sequence hashing, report-time integrity verification, and tampered/missing/extra row
  rejection;
- atomic/idempotent quote, comparison, and report output, deterministic IDs, and stable exit classes;
- report exit-class propagation before and after filters so partial input cannot become success;
- distinct output identities for different quote, compare, or report executables with the same
  package, model, and inputs; and
- multi-transaction EIP-7702 code-identity reconciliation in the quote core.

CI uses only checked-in small protocol fixtures. Live SP1 or RISC0 collection is a separate bounded
campaign step and is not required for deterministic tests.

## Promotion Boundary

Merging these tools means the repository can compute source-independent zkGas quotes and evaluate
them against separately collected backend observations. It does not activate the model in a node,
set a zkGas limit, establish prover capacity, or authorize a mainnet admission change.

Promotion requires a later explicit decision against the draft evaluation standard using declared
natural and controlled corpora, matching backend profiles, compatibility results, and a capacity
policy. Until then, `bin/zkgas/` remains evaluation and regression tooling.
