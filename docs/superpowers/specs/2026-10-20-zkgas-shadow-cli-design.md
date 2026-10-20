# zkGas Quote and Comparison CLI Design

## Status and Boundary

Approved direction: ship a standalone CLI and test framework for computing and evaluating zkGas.
It is not a node admission component, prover-runtime integration, service, or deployment. Promotion
of the pricing model into a real product is a separate future decision made after enough evidence is
collected.

The CLI is source-agnostic. Any producer may supply a valid current-format `GuestInput`: a public
SP1 task, a local SP1 run, a RISC0 run, a saved fixture, or another source. Succinct is not a trust
boundary or dependency of the zkGas evaluator.

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

- Do not fetch Succinct, RISC0, RPC, explorer, or cluster data inside the evaluator.
- Do not reconstruct `GuestInput` from proposal or block ranges.
- Do not run an SP1 or RISC0 guest to obtain truth.
- Do not add a raiko2 server endpoint, background job, config flag, or runtime persistence path.
- Do not change headers, difficulty, transaction admission, proof requests, or proving behavior.
- Do not make a production-admission, capacity, or promotion claim from a comparison report.
- Do not support historical `GuestInput` schema translation in the initial version.
- Do not commit downloaded corpora or generated campaign reports to this repository.

## Tool Composition

The workflow has three independent roles:

```text
input producer ──> GuestInput bytes ──> zkgas-shadow quote ──> quote rows
truth producer ──> workload observations ────────────────────> compare/report
```

An input producer owns acquisition and retention. A truth producer measures or retrieves backend
workload. For today's daily SP1 campaign, the existing public-task collector may perform both roles.
A future RISC0 collector emits the same observation schema with a different backend, unit, and
program identity.

The evaluator checks byte identity, internal `GuestInput` consistency, join keys, units, and profile
compatibility. It does not prove that a producer measured the claimed input; that relationship is an
explicit assertion in the observation row and remains part of corpus provenance.

## CLI

Add a workspace binary named `zkgas-shadow`.

### Quote one input

```text
zkgas-shadow quote \
  --input-bincode ./guest-input.bin \
  --corpus-id ad-hoc-check \
  --sample-id mainnet-proposal-123 \
  --output ./quote.json
```

`quote` accepts exactly one current-format canonical bincode `GuestInput`. JSON may be supported as
a developer convenience, but the CLI re-encodes it and identities always refer to canonical bincode.
The command calls `raiko2-zkgas-trace::quote_shasta_proposal`; source-specific acquisition logic is
not linked into the binary.

### Quote a corpus

```text
zkgas-shadow quote-batch \
  --inputs ./corpus/manifest.json \
  --output ./quotes.jsonl
```

Each input-manifest row supplies a unique `sample_id`, a manifest-relative bincode path, SHA-256,
and byte length. Paths are normalized and must remain beneath the manifest directory. Symlinks,
non-regular files, absolute paths, and parent traversal are rejected. Rows are processed with
bounded concurrency and deterministic output ordering. Every manifest row produces either a
complete quote row or a typed failure row; failures are never silently dropped.

The quote commands read but do not copy, download, delete, or retain input artifacts. Corpus storage
is deliberately owned by the input producer. Reports bind the manifest hash and input hashes so a
caller can determine whether the referenced corpus is still available. A single-quote output and a
batch output are written atomically. If the exact output path exists, the CLI reuses it only when its
bytes match the newly computed result; otherwise it fails rather than overwriting it.

### Compare with backend observations

```text
zkgas-shadow compare \
  --quotes ./quotes.jsonl \
  --observations ./sp1-observations.json \
  --config ./comparison.toml \
  --output ./comparison-store
```

The quote foreign key is `(corpus_id, sample_id, guest_input_sha256)`. Each observation references
that key but has its own primary key `(observation_set_id, observation_id, attempt_id)`, so one quote
may join to any number of SP1, RISC0, or repeated observations. Duplicate observation primary keys
or conflicting quote keys fail closed; unmatched quote and observation rows remain visible in the
output accounting. The joined-row identity additionally contains the compatibility-profile ID.

The content-addressed comparison config declares compatible backend/program profiles, optional
normalization artifacts, and any predeclared workload buckets. An embedded initial SP1 profile is
available for the current pricing model; a file override is explicit and recorded in the report.

Optional filters such as `--chain-id`, `--backend`, and `--program-id` restrict a run explicitly and
are written into the report. Without a filter, heterogeneous rows remain in the output but are
stratified; they are never silently combined into one compatibility result.

The existing `guest-launcher proposal-workload-quote` developer interface remains compatible in
this PR. Both interfaces use the same quote implementation. Removing that interface is a separate
compatibility decision.

## Portable Input Contracts

### Input manifest row

The input manifest has a strict `schema_version`, a producer-chosen `corpus_id`, and an ordered
sample list. Unknown top-level or row fields are rejected; arbitrary producer data is allowed only
inside the bounded `metadata` object. An input row contains:

- `sample_id`: producer-assigned stable identifier, unique within the manifest;
- `guest_input.path`: path relative to the manifest;
- `guest_input.sha256` and `guest_input.byte_length`; and
- optional opaque producer metadata that is preserved but not interpreted.

The evaluator opens each path without following symlinks, verifies it is a regular file still
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

The evaluator treats `source` as provenance, not authority. It never contacts that source. Program
identifiers with different algorithms are stored separately and are never compared as though they
were the same digest.

## Quote Identity and Validation

Each complete quote row contains the full `ProposalWorkloadQuote` and an evaluation envelope with:

- schema version and required evaluator executable SHA-256;
- CLI package version plus optional source revision and dirty marker as readable provenance;
- `corpus_id`, `sample_id`, canonical GuestInput SHA-256, and byte length;
- pricing `model_id` and output schema identity;
- `taiko.proposal_id`;
- manifest chain name and chain ID;
- proof-carry chain ID and proposal ID;
- the canonical hash of every witness chain spec;
- first and last block number, block count, and public output;
- the ordered active-fork segments across that block range; and
- status and a typed failure when validation does not complete.

The evaluator requires the manifest, proof-carry data, and every witness to agree on chain and
proposal identity wherever the current data model carries the same fact. It also verifies contiguous
block topology through the canonical proposal-validation path. A disagreement is a typed failure,
not a separate stratum.

This identity prevents mainnet, Hoodi, devnet, and Masaya samples from being confused even when they
share a requester, program, or block number.

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

Direct prediction-error metrics are enabled only when all of these match a declared compatibility
profile. For the initial model that means:

- prediction is `quote.prove.total_pgu` with unit `sp1_pgu`;
- observation unit and stage are the profile's SP1 PGU truth surface; and
- the observed program identity matches the pricing calibration identity using the same digest
  algorithms.

The pricing calibration identity is provenance-bound to the pinned Alethia Reth
`runtime-spec.json` and `RUNTIME_SPEC_SHA256`. Its production ELF SHA-256, serialized-VK artifact
SHA-256, and any reviewed binding to an SP1 vkey digest remain separately named fields.

For the same unit but an incompatible program, `compare` may expose the numerical ratio only as a
cross-program diagnostic and must not classify it as model error, overquote, underquote, or a safety
result. For a different unit, it does not compute a raw ratio. Both cases may report scale-invariant
workload-shape metrics such as raw-value Pearson, Spearman, Kendall, and predeclared bucket
amplification. This permits future RISC0 comparison without pretending RISC0 cycles and SP1 PGU are
the same unit.

An optional immutable normalization profile may map one named score unit and program profile to one
named observation unit and program profile. Normalized results are always labeled diagnostics. The
profile preserves the original calibration artifact bytes and hash; a wrapper records the exact
lossless rational interpretation used by the CLI as a separate identity.

For the existing Unzen scalar, the only proposal-level conversion is:

```text
calibrated_unzen = ceil(unzen_proposal_sum * numerator / denominator)
```

It uses checked integer arithmetic and never sums per-block rounded values or parses through binary
floating point.

## Comparison Output

`--output` names a comparison store root. Each run creates an immutable directory containing:

```text
runs/<run-id>/manifest.json
runs/<run-id>/rows/<join-key-sha256>.json
runs/<run-id>/report.json
```

The run ID is the SHA-256 of a canonical descriptor binding the exact quote, observation, and
comparison-config file hashes, filter set, required evaluator executable SHA-256, model/profile
identities, arithmetic policy, and ordered join keys. Package version and source revision do not
substitute for the executable content identity. Output filenames are derived from hashes, never
directly from producer-controlled IDs. Files are written through a staging directory and committed
atomically. Reusing an existing run ID byte-validates every file and never silently overwrites a
conflict.

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
counted separately and excluded from ratio quantiles. Display ratios use six fractional digits and
round half to even; classifications use unrounded integers.

For every homogeneous stratum, the report states the comparison-eligible population and computes:

```text
count_weighted_prediction_over_actual = sum_i(prediction_i / actual_i) / row_count
workload_weighted_prediction_over_actual = sum_i(prediction_i) / sum_i(actual_i)
underquote_rate = count_i(prediction_i < actual_i) / row_count
```

Quantiles use nearest rank with the convention and population recorded. Strata include at least:

- chain ID and active fork;
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
quote overflow, incompatible units/profiles, invalid normalization, and output conflict.

Batch input count, individual input bytes, total bytes read, concurrency, and processing time have
explicit limits. Default concurrency is one because native replay is memory-intensive. Each input is
released before the next begins; the command never loads the full corpus into memory.

A row-level failure does not erase other results, but a structurally invalid manifest, ambiguous
join, or inability to commit the report returns nonzero. Partial results remain explicitly marked.

Exit classes are stable for automation:

- `0`: the requested output was committed and every requested row completed; a correctly classified
  incompatible profile or unavailable metric is not a processing failure;
- `2`: a structurally valid output was committed with one or more failed or unmatched rows, including
  the case where every row failed; and
- `1`: usage/configuration error or no structurally valid output could be committed.

## Verification

Implementation requires focused automated coverage for:

- quote equivalence between `zkgas-shadow` and the existing guest-launcher entry point;
- strict input/observation schema versions and bounded opaque metadata;
- current bincode canonical round-trip and invalid-input failure;
- exact input hash/length checks plus absolute, parent-traversal, symlink, and special-file rejection;
- proposal ID, chain ID, chain-spec, active-fork, block-range, and public-output extraction;
- mixed mainnet/Hoodi/dev/Masaya fixtures and mismatch rejection;
- deterministic batch ordering, bounded memory, and typed partial rows;
- one-to-many quote/observation joins, repeated-attempt identity, duplicate conflict, and unmatched
  accounting;
- SP1 compatible-unit direct metrics, zero prediction, zero actual, rounding, and quantiles;
- RISC0 or synthetic incompatible-unit rows that permit only scale-valid recognition metrics;
- program/digest algorithm mismatch and calibration incompatibility labeling;
- exact Unzen proposal-sum normalization and a guard against raw Unzen/backend ratios;
- homogeneous stratification and prevention of cross-chain/program/backend aggregation;
- atomic/idempotent quote and report output, deterministic run IDs, and stable exit classes;
- distinct quote/run identities for different evaluator executables with the same package, model,
  and inputs; and
- multi-transaction EIP-7702 code-identity reconciliation in the quote core.

CI uses only checked-in small protocol fixtures. Live SP1 or RISC0 collection is a separate bounded
campaign step and is not required for deterministic tests.

## Promotion Boundary

Merging this CLI means the repository can compute source-independent zkGas quotes and evaluate them
against separately collected backend observations. It does not activate the model in a node, set a
zkGas limit, establish prover capacity, or authorize a mainnet admission change.

Promotion requires a later explicit decision against the draft evaluation standard using declared
natural and controlled corpora, matching backend profiles, compatibility results, and a capacity
policy. Until then, this CLI remains evaluation and regression tooling.
