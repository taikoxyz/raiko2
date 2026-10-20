# zkGas evaluation tools

`zkgas-quote`, `zkgas-compare`, and `zkgas-report` are local developer tools for evaluating
proposal workload pricing. They exchange files and may be used independently. Pricing stays in
`raiko2-zkgas-trace::quote_shasta_proposal`; quote performs native proposal replay, while compare
and report consume saved results.

Input acquisition and workload measurement belong to external producers. Supply a valid current
Shasta `GuestInput` from a retained fixture, local execution, public task, or another source, and
separately supply observations. These tools never fetch RPC, explorer, Succinct, RISC0, or cluster
data, reconstruct historical inputs, translate old input schemas, run guests, or build ELF assets.
They add no server configuration or deployment behavior. Comparison checks the asserted join and
compatibility, **not the truth of a producer's measurement**. Reports make no capacity, admission,
or model promotion claim.

## Build and command workflows

From the repository root:

```sh
cargo build -p zkgas --bins
target/debug/zkgas-quote --help
target/debug/zkgas-compare --help
target/debug/zkgas-report --help
```

1. Quote one input or an ordered input manifest into quote JSONL.
2. Compare one or more quote JSONL files against one or more observation JSON manifests, producing
   an immutable comparison run. Acquisition and measurement happen before this step.
3. Report a verified comparison run using a strict JSON or TOML config. Regenerate reports without
   replaying inputs or changing row-level compatibility decisions.

### Quote: JSON, canonical bincode, or batch

Single-input mode requires `--corpus-id` and `--sample-id`. Exactly one input mode is allowed:

```sh
target/debug/zkgas-quote \
  --input-json tests/fixtures/shasta_guest_input_taiko_mainnet_proposal_23077_l2_9051439_9051630.json \
  --corpus-id example-corpus --sample-id proposal-23077 --output quote-json.jsonl

target/debug/zkgas-quote --input-bincode corpus/input.bin \
  --corpus-id example-corpus --sample-id proposal-23077 --output quote.jsonl

target/debug/zkgas-quote --manifest corpus/manifest.json --output quotes.jsonl
```

The JSON convenience mode deserializes exactly the supplied current-format input, without hidden
proof-carry backfill, and serializes canonical bincode for identity. The bincode mode requires a
byte-identical canonical round trip. Both quote keys use canonical bincode SHA-256 and length,
even if the original input was JSON. A JSON file's raw SHA-256 is not its quote-key hash.

For batch mode, save an input manifest like this as `corpus/manifest.json`, replacing the digest
and length with those of the canonical bincode file (`sha256sum corpus/input.bin` and
`wc -c < corpus/input.bin`):

```json
{
  "schema_version": 1,
  "corpus_id": "example-corpus",
  "samples": [{
    "sample_id": "proposal-23077",
    "guest_input": {
      "path": "input.bin",
      "sha256": "REPLACE_WITH_CANONICAL_BINCODE_SHA256",
      "byte_length": 123
    },
    "metadata": {"kind": "retained input fixture"}
  }]
}
```

Manifest paths must be relative and confined beneath the manifest directory. Absolute paths,
parent traversal, symlinks in any component, and non-regular files are rejected using a confined
Linux `openat2` open. Declared lengths/counts are preflighted before corpus replay. Samples execute
sequentially in manifest order, dropping each input before decoding the next. Each semantically
bad sample becomes a typed `failed` quote row; structural manifest failures produce no output.
Every complete row includes the full quote, executable identity, canonical input identity,
proposal/public-output identity, chain-spec hashes, and ordered active-fork segments. A failed
row records verified identity only when available, separately from any observed raw-file hash.

### Compare: compatible SP1 example

`--quotes` and `--observations` are repeatable. `--profiles` replaces the embedded profiles; it
does not add to them. Without that option, the embedded SP1 calibration profile and sealed Unzen
normalization are used.

The following is a **synthetic schema example**, not measured SP1 truth. It requires `jq` and a
complete single-row `quote.jsonl` from the preceding command. It deliberately asserts the known
calibration identity and invents workload `100`; never pass this off as execution evidence:

```sh
jq -s '.[0] as $q | {
  schema_version: 1,
  observation_set_id: "synthetic-examples",
  observations: [{
    status: "complete",
    observation_id: "synthetic-sp1",
    attempt_id: "example-1",
    quote_key: $q.key,
    backend: "sp1",
    workload: {value: "100", unit: "sp1_pgu"},
    measurement_stage: "execute",
    program: {
      elf_sha256: "ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7",
      serialized_vk_sha256: "bd71c1f1cbd72387aa057ae6e936afd6dcb0bc170c6a91193c6d1a0b47507102"
    },
    source: {kind: "synthetic example; no guest execution or measurement"}
  }]
}' quote.jsonl > synthetic-sp1.json
```

Save this as `example-profiles.toml`. It contains exactly one matching profile and omits optional
normalization, so no external normalization artifact is needed. For a different quote model,
review the intended compatibility and explicitly bind `model_id` and `runtime_spec_sha256` to
that quote; changing a model or program label alone does not establish empirical calibration.

```toml
schema_version = 1

[[profiles]]
profile_id = "synthetic-calibration-example"
model_id = "93d04eddda3779393807808859eb36561d35aad0cdd8a8a3960ec3e279a93046"
runtime_spec_sha256 = "93d04eddda3779393807808859eb36561d35aad0cdd8a8a3960ec3e279a93046"
backend = "sp1"
prediction_unit = "sp1_pgu"
observation_unit = "sp1_pgu"
measurement_stage = "execute"

[profiles.program]
elf_sha256 = "ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7"
serialized_vk_sha256 = "bd71c1f1cbd72387aa057ae6e936afd6dcb0bc170c6a91193c6d1a0b47507102"
```

```sh
target/debug/zkgas-compare --quotes quote.jsonl --observations synthetic-sp1.json \
  --profiles example-profiles.toml --output comparison-store
```

Compare prints the run ID and writes
`comparison-store/runs/<run-id>/{manifest.json,rows/<comparison-row-id>.json}`.
Quote keys are `(corpus_id, sample_id, guest_input_sha256)`; observation keys are
`(observation_set_id, observation_id, attempt_id)`. Many unique observations may reference one
quote, including repeated attempts. Any repeated primary key, even byte-identical across files,
is a structural error. Unmatched rows remain visible rather than being silently dropped.

Before any join or metric, compare revalidates every duplicated complete-quote invariant: canonical
input hash/length, proof-carry and proposal identities, contiguous block and fork envelopes, public
output, pricing unit/surface/model, and the embedded node/prove sums and fixed-point totals. An
inconsistent complete row, or an empty quote or observation population, is a structural failure and
publishes no run.

### Compare: cross-program SP1 and incompatible-unit RISC0

For real observations, record the actual ELF SHA-256, backend-native identifier, stage, unit,
versions when known, and truthful source provenance. The current packaged SP1 ELF may differ
from the calibration ELF above. Such a run is diagnostic-only until the identities match; an
`execute` result does not become compatible merely because both programs use SP1.

These additional synthetic examples preserve the verified quote key while changing the asserted
program or unit. They do not execute either backend:

```sh
jq '.observations[0].program.elf_sha256 =
  "0000000000000000000000000000000000000000000000000000000000000000" |
  .observations[0].observation_id = "synthetic-cross-program-sp1"' \
  synthetic-sp1.json > synthetic-cross-program-sp1.json

target/debug/zkgas-compare --quotes quote.jsonl \
  --observations synthetic-cross-program-sp1.json \
  --profiles example-profiles.toml --output comparison-store

jq '.observations[0] |= (.backend = "risc0" |
  .observation_id = "synthetic-risc0" |
  .workload = {value: "100", unit: "risc0_cycles"} |
  .program = {
    elf_sha256: "1111111111111111111111111111111111111111111111111111111111111111",
    risc0_image_id: "synthetic-image-id"
  })' synthetic-sp1.json > synthetic-risc0.json

target/debug/zkgas-compare --quotes quote.jsonl --observations synthetic-risc0.json \
  --profiles example-profiles.toml --output comparison-store
```

Both runs are complete with one `incompatible_profile` row. Cross-program SP1 has only a
`cross_program_ratio` diagnostic. The RISC0 row has no raw ratio because `risc0_cycles` and
`sp1_pgu` are different units. Neither row is eligible for direct underquote classification.

### Report and regenerate without replay

Save this as `report-config.json`:

```json
{
  "schema_version": 1,
  "filters": {},
  "quantiles": ["0.5", "0.95", "1"],
  "buckets": [{"name": "example-range", "minimum": "0", "maximum_exclusive": "1000000"}]
}
```

```sh
target/debug/zkgas-report --comparisons comparison-store/runs/<run-id> \
  --config report-config.json --output report.json
```

Replace `<run-id>` with the ID printed by compare. For regeneration, rerun this command with a
changed config and a new output path. Supported optional filter arrays are `chain_ids`,
`backends`, `program_elf_sha256`, `model_ids`, and `profile_ids`. Missing filter dimensions do not
match constrained filters. The entire store is integrity-checked before filtering; an excluded
row cannot hide a corrupt file or an upstream partial run.

Reports retain whole-run counts, selected counts, and exclusions with reasons. They always group
by chain ID, ordered fork topology with transition offsets relative to the proposal's first block,
backend, workload unit, stage,
full program identity, backend version, model, resolved profile, compatibility, and normalization
identity. Absolute block numbers therefore do not split ordinary same-fork proposals, while a
proposal that crosses a fork (and the relative position of that transition) remains an explicit
stratum distinction. Omitting filters never combines heterogeneous strata. Compatible positive-actual rows
are eligible for count-weighted ratios, nearest-rank quantiles, and underquote rate. Zero-actual
rows are accounted separately; workload-weighted ratios use full compatible stratum sums when
the total actual is positive. Zero prediction with positive actual is infinite underpricing.
Incompatible strata expose series-shape diagnostics only: Pearson, average-tie Spearman,
Kendall tau-b, and predeclared prediction-bucket actual-workload amplification. Insufficient or
constant series carry explicit unavailability reasons. No cross-unit direct ratio is created.

## Contracts, units, and compatibility

The authoritative types are in [`src/contracts.rs`](src/contracts.rs). All eight contracts have
independent `schema_version = 1`, reject other versions and unknown fields, and require their
documented fields. Arbitrary producer fields belong only in bounded `metadata` or `source`
objects. JSON workloads and quote `u128` quantities are unsigned decimal strings to preserve
integer precision; input byte lengths, chain IDs, and row counts are JSON numbers.

| Contract | Producer → consumer | Version | Units/surface | Compatibility rule |
| --- | --- | --- | --- | --- |
| Input manifest (JSON) | Input producer → quote | 1 | Canonical bincode bytes | Unique sample IDs; confined regular files; exact SHA-256/length |
| Observation manifest (JSON) | External workload producer → compare | 1 | Declared `workload.unit` and `measurement_stage` | Unique observation keys; explicit quote foreign key and algorithm-named program identity; tagged complete/failed rows |
| Quote row (JSONL) | Quote → compare | 1 per row | `sp1_pgu`, `prove` prediction | Canonical bincode key, validated proposal envelope, model identity; tagged complete/failed rows |
| Compatibility profiles (TOML) | Calibration owner → compare | 1 | Explicit prediction/observation units and stage | Exactly one matching model/backend/unit/stage/program profile; optional version/runtime-spec constraints |
| Comparison manifest (JSON) | Compare → report | 1 | No implicit conversion | Immutable run ID binds ordered input/config/executable identities, arithmetic policy, row IDs/hashes and sequence |
| Comparison row (JSON) | Compare → report | 1 | Compatible direct metrics or labeled diagnostics | Tagged result, embedded quote/observation, profile decision, domain-separated row ID |
| Report config (JSON/TOML) | Analyst → report | 1 | Explicit filters, decimal quantiles, prediction-unit buckets | Strict bounded config; sorted non-overlapping buckets; no heterogeneous merge option |
| Report output (JSON) | Report → analyst | 1 | Metrics within homogeneous strata | Verified complete store, config/executable identity, whole-run/selected accounting, immutable report ID |

Only `quote.prove.total_pgu` is the complete new-model SP1 PGU prediction. `node` is informational;
its raw Unzen `unzen_zkgas_sum` is not SP1 PGU. Direct compatibility currently requires `prove`
prediction in `sp1_pgu`, an SP1 observation in `sp1_pgu`, the matching quote model/calibration
identity, and a single profile matching the observation's stage and every program algorithm
named by that profile. `elf_sha256`, `serialized_vk_sha256`, `sp1_vkey`, and `risc0_image_id` are
separate algorithms; one cannot substitute for another. Additional observed algorithms are
retained as provenance. Optional backend-version/runtime-spec constraints must also match.
Zero matches yield one `incompatible_profile` row; multiple matches yield one `ambiguous_profile`
row, not one row per candidate.

Optional Unzen normalization remains diagnostic-only. A supplied profile referencing it must
also supply the artifact at its confined config-relative `artifact_path`; raw artifact SHA-256
and its separate internal artifact identity are both verified. All named source/target program
and unit constraints must match. Conversion is exactly
`ceil(proposal_unzen_sum * numerator / denominator)`, once per proposal, using arbitrary-precision
integers. It is not the sum of block-wise rounded conversions. Classification uses exact
integers/rationals before display rounding; displayed ratios use six-place round-half-to-even.

## Resource bounds and exit codes

Limits are hard ceilings. Quote's CLI options may tighten their defaults, never raise them.

| Command/contract | Current bounds |
| --- | --- |
| Quote | `--max-manifest-bytes` 16 MiB; `--max-input-bytes` 512 MiB; `--max-total-bytes` 8 GiB; `--max-rows` 10,000; `--max-elapsed-seconds` 3,600; `--max-output-bytes` 64 MiB; `--concurrency` exactly 1 |
| Compare | 128 input/config/artifact files; 64 MiB per quote/observation file; 256 MiB aggregate input; 10,000 total quote plus observation rows; 1 MiB per profile config/artifact; 128 profiles; 256 MiB serialized output |
| Report | 10,000 rows; 4 MiB manifest; 8 MiB per row; 256 MiB manifest plus rows; 64 KiB config; 128 entries per config collection; 32 MiB output |
| Opaque `metadata`/`source` | 65,536 serialized JSON bytes, depth 16, 1,024 total object/array entries per object |

Quote checks the elapsed budget before and after each native replay; it does not interrupt an
in-progress replay. Batch quote rows stream into a sibling staging file under the 64 MiB compare
input ceiling, retaining only the current serialized row; a limit failure removes staging and
publishes nothing. Report config additionally bounds semantic depth to 16, nodes to 4,096, and
keys/strings to 1,024 bytes. Quantiles are unique numeric decimals in `[0,1]` with at most 18
fractional digits and 32 bytes total; bucket boundaries are `u128` decimal strings of at most
39 digits, names are at most 128 bytes. Normalization numerator/denominator strings are bounded
to 1,024 digits and the denominator must be positive.
Report JSON receives a lexical allocation gate before typed parsing (also used for store JSON,
with 64 nesting levels, 250,000 tokens, 128-byte primitive tokens, and 10,000 elements per array
for store documents or 128 for config). Raw JSON string contents are bounded to 65,536 bytes in
store documents or 1,024 in config, before escape decoding. TOML's authoritative parser first parses the
whole document under the raw byte ceiling; semantic limits apply afterward. This distinction
also applies to profile TOML's raw 1 MiB bound and post-parse 128-profile check: do not assume an
early row-limit error takes precedence over a malformed TOML tail.

Report's per-row/output ceilings are intentionally smaller than compare's aggregate ceiling;
a comparison containing an exceptionally large quote envelope may therefore be rejected by
report. Kendall diagnostics use bounded quadratic pair counting, so large strata can be costly.

| Exit | Meaning |
| --- | --- |
| `0` | Complete committed output or identical reuse; help also exits 0. Incompatible-only comparison/report runs are complete. |
| `1` | CLI, structural input/resource, integrity, or output-conflict error; no new valid result is committed. Existing outputs remain. |
| `2` | Valid partial output was committed (or reused). Quote has typed failed rows; compare has unmatched, invalid/upstream-failed, or ambiguous rows; report preserves the comparison run's partial status even after filtering. |

## Immutable identities and reuse

Single quotes bind a canonical invocation descriptor without a filesystem path. Batch quotes bind
the exact raw manifest hash and row ordinal. Every executable is identified by SHA-256 of its
actual running image bytes (a pinned `/proc/self/exe` file on Linux), plus readable
package/revision/dirty provenance when available. Rebuilding a binary
can therefore change identities even if its behavior is unchanged.

Comparison IDs bind ordered raw quote/observation hashes, raw profile config hash, executable,
models/profiles, arithmetic policy, and all ordered row IDs and row-byte hashes. Report IDs bind
the comparison manifest/sequence, raw report config, report executable, filters, and metric/grouping
policy. Whitespace-only raw config/manifest edits can change identity without changing math.
Generated descriptors use canonical JSON with sorted object keys and preserved array order;
identity domains distinguish joined, incompatible, unmatched, and other row categories.

Outputs are published atomically. An existing quote/report file or comparison run is reusable
only if all expected bytes match; a conflicting, tampered, extra-file, or partial existing output
is rejected, never overwritten or repaired automatically. Report verification checks the exact
manifest/row filename set, row hashes/IDs, run ID, sequence hash, status, and accounting before
building aggregates. Preserve source inputs and observations externally if future reproduction
is required: the tools do not copy or retain a corpus.

## Deterministic regression

```sh
cargo test -p zkgas --test end_to_end
cargo test -p zkgas
cargo clippy -p zkgas --all-targets -- -D warnings
```

The end-to-end test canonicalizes the checked-in proposal fixture in a temporary directory and
invokes the actual three binaries. Its SP1/RISC0 observations are explicitly synthetic fixtures.
It checks public-output parity, immutable hashes, compatibility/metric eligibility, homogeneous
strata, complete/partial exits, filtered accounting, and deterministic reuse. It performs no guest
execution; a real packaged-SP1 measurement is a separate external-producer validation step.
