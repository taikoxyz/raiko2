# Reproducible workload experiments

These tools retain finite ProgramRequest experiments and measure saved ordinary GuestInputs.
The objective is **actual SP1 PGU / node-known PGU**. Complete quotes and paired differences
are diagnostics. Controls, native rejections, failed measurements and unattempted cases stay
visible. A proposal's actual PGU cannot be assigned to individual blocks.

The sampler executes locally with SDK6.3.0 and an explicit ELF. It never proves, fetches data,
selects an embedded program, patches an input or establishes production SDK parity. Raiko's
normal workspace SDK and production launcher remain independent. The ELF must understand the
input schema: qualify a real native control first, then require identical input identities and
public outputs. Preserve an incompatibility as a failed observation; select a compatible ELF
explicitly in a new run configuration.

For guarded measurements the sampler receives `--resource-receipt sp1-resources.json`. Before
the first execution chunk it records the four newly owned SDK shared-memory/semaphore objects
using names derived from its open descriptors plus exact device/inode/UID identities. After
every exit the supervisor waits for owned descendants to stop, then removes only still-matching
receipted objects; normal SDK cleanup already removes them on successful execution. Cleanup
results remain in the stage outcome. This prevents SIGKILL from retaining heavy shared-memory
pages across failed attempts. Objects outside the receipt are never deleted. Termination during
constructor/receipt publication can leave small initial headers or semaphore objects; a missing
or corrupt receipt records incomplete cleanup and requires investigation before retrying.

## Build and check

Run from the repository root. Use an existing bounded Cargo target directory when local disk is
small; these commands can otherwise build substantial dependencies. No guest build is needed.

```sh
cargo build --release --locked -p zkgas --bin zkgas-quote
cargo build --release --locked -p zkgas --features fixture --bin zkgas-fixture
cargo test --locked -p zkgas --features fixture --test fixture
cargo build --release --locked --manifest-path bin/zkgas/sp1-estimator/Cargo.toml
cargo test --locked --manifest-path bin/zkgas/sp1-estimator/Cargo.toml
~/.venv/bin/python -m unittest discover -s bin/zkgas/bench/tests -v
```

The optional fixture feature supplies the native constructor. Default quote/report builds do
not require the constructor or independent sampler. The sampler has its own manifest and
Cargo.lock, pins SDK/executor/runner6.3.0, and tests the raw one-buffer stdin framing against
`SP1Stdin::write(value)`. Do not pass serialized `SP1Stdin` or serialize the input bytes again.
Input and ELF must each be regular files of at most64MiB. Output uses exclusive creation;
failure may retain an empty report reservation, which must never be mistaken for success.

For source attribution, set `ZKGAS_SOURCE_REVISION` to the reviewed source revision when
building. Reports include the executable/program/input hashes, manifest and lock hashes,
complete executor options, SDK identities, PGU, instructions, syscalls and raw public values.

## Frozen configuration

Create a private operator configuration, for example `experiment/config.json`. Artifact paths
are relative to that file. Every SHA256 below must be replaced with the lowercase64-digit hash
of the actual file; the harness rejects placeholders. Use `sha256sum` to inspect identities.
Supply `model` as the artifact used to build the selected quote binary. Its file hash and
top-level `artifact_sha256` metadata (equal to `model_id`) are checked. Quote authority is bound
by the frozen executable hash and its reported model ID; the harness does not reimplement
the model's content-addressing or derive embedded binary data from that separate file.
Changing either identity requires a fresh root.

```json
{
  "schema_version": 1,
  "corpus_id": "copy-frame-control-v1",
  "model_id": "EXPLICIT_MODEL_ID",
  "artifacts": {
    "native": {"path": "tools/zkgas-fixture", "sha256": "REPLACE_WITH_SHA256"},
    "quote": {"path": "tools/zkgas-quote", "sha256": "REPLACE_WITH_SHA256"},
    "sp1": {"path": "tools/zkgas-sp1-estimator", "sha256": "REPLACE_WITH_SHA256"},
    "elf": {"path": "programs/proposal.elf", "sha256": "REPLACE_WITH_SHA256"},
    "model": {"path": "models/model.json", "sha256": "REPLACE_WITH_SHA256"}
  },
  "limits": {
    "pss_mib": 6144,
    "min_free_mib": 256,
    "output_mib": 192,
    "log_mib": 16,
    "timeout_seconds": 600
  },
  "provenance": {"source_revision": "EXPLICIT_REVIEWED_REVISION"}
}
```

The harness freezes identities and resources in `contract.json`, rechecks tools during each
attempt, and serializes access to one output root with a lock. Run only one estimator across
all output roots on a host. Operator-configured directories can live on disk or tmpfs.

For workload tests beyond the legacy Unzen admission cap, use a fresh root with the explicit
configuration field `"execution_profile": "evm-gas-only"` and an additional entry under
`artifacts`:

```json
{
  "program_manifest": {
    "path": "programs/program-manifest.json",
    "sha256": "REPLACE_WITH_SHA256"
  }
}
```

The manifest belongs to the reviewed experimental program and must contain:

```json
{
  "schema_version": 1,
  "execution_profile": "evm-gas-only",
  "elf_sha256": "REPLACE_WITH_ELF_SHA256",
  "sp1_sdk_version": "6.3.0"
}
```

Additional source/build provenance is retained in the manifest. Its hash, profile, SDK and
ELF identity are checked before sampling; the native constructor, complete quote and SP1
report must agree with the frozen profile. Native and quote commands receive the same
`--execution-profile` value; the sampler receives `--program-manifest`. The GuestInput stays
one opaque canonical bincode buffer. A manifest binds a program's declared policy and bytes;
it does not inspect arbitrary ELF behavior. Qualify the reviewed experimental program with
both previously admitted controls and cases that exceed the old cap.

`evm-gas-only` retains EVM gas and complete proposal validation while removing the legacy
100M admission boundary from the workload experiment. It does not activate new pricing in
production. Legacy rejection alone is not coverage of inputs the new model could admit.
Keep observations from different program hashes separate and compare paired controls under
the same program. Do not combine old-program controls with a new program's candidates.

Configurations without `execution_profile` keep their existing frozen contract and legacy
report compatibility. Explicit `legacy-unzen` is also supported. An absent profile in an old
report is accepted only as legacy. Without a manifest the opaque sampler makes no policy
claim; accepting such a program for legacy measurement remains the operator's choice.
Changing the profile, ELF or manifest requires a fresh output root.

Memory accounting requires Linux `/proc` and includes sampled full descendant plus runner
PSS; the ceiling is6GiB. The SDK's24GiB VM policy has different semantics and remains frozen.
Do not equate the two limits. The minimum disk floor is256MiB, with320MiB additional reserved
headroom before an attempt. Sampled output/disk/time guards bound ordinary artifacts; a
supervisor pipe strictly caps retained log bytes. Native/quote stages also have a kernel
per-file cap. SP1 omits that cap because the SDK uses large sparse shared-memory reservations
through `ftruncate`, which Linux also subjects to `RLIMIT_FSIZE`; resident pages remain under
the PSS guard. Ordinary SP1 report size and disk use are sampled, so they may briefly overshoot
their configured bounds between checks.
Choose timeouts for the finite cohort; a timeout is retained, never retried automatically.

## Generate and screen a finite catalog

Case generators under `cases/` emit ordinary ProgramRequest files and a catalog. A catalog
contains metadata without an executable vendor schema:

```json
{
  "schema_version": 1,
  "cases": [
    {
      "sample_id": "control-stop-b1",
      "request_path": "requests/control-stop-b1.json",
      "request_sha256": "REPLACE_WITH_SHA256",
      "role": "control",
      "pair_group": "copy-mode-b1"
    }
  ]
}
```

Request paths are relative to the catalog. IDs must be unique portable path components.
Generation is separate from measurement: preserve the generated catalog, requests and compiler
artifacts before running. When adding a case, write one ProgramRequest using the native
constructor's supported fields, include expected observed opcode/terminal counts, add its hash
and metadata to the catalog, and include a matched control. Freeze transaction gas, input,
bytecode and memory-layout envelopes when comparing one variable; document intentional changes.
The initial producer covers one user transaction per block, not arbitrary multi-user state.

For the retained CALL output-memory boundary cohort, use an explicit Solidity0.8.21 compiler:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/spawn_memory.py \
  --solc /path/to/solc --output-dir experiment/spawn-memory
```

This emits44 requests, compiler source/settings/output and a catalog at
`experiment/spawn-memory/catalog.json`. Four CALL variants have matched zero-length controls;
the leaf returns no data and each iteration creates fresh caller memory. Count/status gates
exclude early failure. Bytecode is identical across the cohort; calldata changes the requested
length, repetition or CALL variant. Use this catalog with the following `screen`/`run` commands.
See the [selector ownership investigation](../../../docs/research/2026-10-29-call-memory-selector-ownership.md)
for the motivating observation and its evidence limits.

The complementary input-memory cohort uses the same compiler and completion gates:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/spawn_input.py \
  --solc /path/to/solc --output-dir experiment/spawn-input
```

Its44 requests expand the middle frame's CALL input range and pass128 header bytes plus
the selected zero-filled length to a leaf that reads only the header. CALL output length
stays zero. This separates input allocation/copy work from output allocation; compare each
family against its own fixed-bytecode zero-length controls. Use the input catalog in a fresh
output root; its sample IDs differ from the output-memory family.

To isolate actual automatic returndata copying from caller memory growth, generate the
90-case preallocated return-copy catalog with the explicitly selected compiler:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/return_copy.py \
  --solc /path/to/solc --output-dir experiment/return-copy
```

Each triplet uses one runtime and fixed 256-byte calldata, changing only CALL output capacity
between zero, half and full. The parent preallocates distinct input/output ranges and guard
words once, then reuses them. Self RETURN and caught REVERT span all four CALL variants;
IDENTITY provides selected negative controls. Successful modes execute the same validation
loads and branches, including alternating first-byte, copy-boundary and untouched-sentinel
checks. Exact native CALL/RETURN/REVERT counts reject early exits; no RETURNDATACOPY is emitted.
The finite grid includes 0/1/31/32/33-byte boundaries, 64 KiB repetition ramps and bounded larger
allocations. Zero/half/full modes with equal capacities at short lengths remain separate rows.
Use the new catalog in a fresh output root, retain resource/native rejections, and compare
actual/node ratios before using paired deltas as diagnostics. Set `SOLC_BIN` and
`NATIVE_FIXTURE_BIN` when running `test_return_copy.py` to qualify the compiler and verify
complete native opcode-map equality for the tiny zero/full pair.

The default `--profile boundary` keeps those 90 cases. For a separate bounded repetition
extension, use a new catalog directory and a fresh harness output root:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/return_copy.py \
  --solc /path/to/solc --profile repetition --output-dir experiment/return-copy-repetition
```

This adds 12 independent rows: 64 KiB self RETURN and caught REVERT at 256 and 512
DELEGATECALL repetitions, each with zero/half/full output capacity. It uses the same runtime
and gates. Retain any native budget rejection; neither profile changes the existing cohorts.

For a small BN254 ECMUL proving-workload capacity probe, generate a separate catalog:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/precompile_workload.py \
  --solc /path/to/solc --output-dir experiment/precompile-workload
```

The eleven cases use one runtime and fixed 128-byte calldata: count, scalar and two expected
coordinates. Two scalar families use 0/1/4/16/32 STATICCALLs to address `0x07` with point
`G=(1,2)`: the group order minus two and `dense254 = (3 << 252) - 1`. A single-call comparison
adds `dense253 = (1 << 253) - 1`. Both dense scalars are below the group order and have 253
set bits; the former has one more bit of length. This tests a value axis as well as repetition,
without claiming an absolute maximum. Expected points come from independent Python affine
scalar multiplication; the group-order-minus-two result is also checked against `-2G`.
Each iteration clears both output words before calling, then checks success, exactly 64
returned bytes and both expected coordinates. Exact native STATICCALL and root RETURN counts
exclude skipped work. Input/output areas are preallocated once and reused. The native tests
qualify all three single-call points and equal executed-opcode maps across scalar variants;
intrinsic calldata and fixture data differences remain visible. Keep the two zero-call controls
and use the same explicit `evm-gas-only` profile and reviewed program manifest for all
measurements. These are initial capacity probes, not gas-saturated maxima: the producer's
15M transaction and 31M block gas budgets are tool settings, not claims about current chain
limits. The generator and runtime accept counts up to 4096, above the maximum successful ECMUL call
count permitted by Osaka's 16,777,216 transaction-gas cap at 6000 gas per call before wrapper
and intrinsic costs. This generator bound therefore does not exclude a later legal
gas-saturation probe; increasing a requested count does not raise the producer's gas budget.
Run progressively with the existing resource guards; a larger actual workload is a
measurement result, not a reason to change tariffs automatically. Set `SOLC_BIN` and
`NATIVE_FIXTURE_BIN` to run the real compiler/native qualification tests.

For a finite MODEXP input-value boundary probe, generate another catalog:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/modexp_values.py \
  --solc /path/to/solc --output-dir experiment/modexp-values
```

The 32 cases share one runtime and call precompile `0x05`. For each byte width
`L=128/256/512`, the base is `2^(8L)-1`, the exponent is one byte with value `0/1/3`,
and the modulus is odd (`2^(8L)-189`), a power of two (`2^(8L-1)`), or even CRT
(`3*2^(8L-2)`). The 27 single-call combinations retain exponent-zero controls;
three zero-call CRT/exponent-three controls and width-512 CRT/exponent-three repetitions
at four and sixteen calls complete the matrix. Every call clears all output words,
checks success and exact returndata length, and compares every word with Python `pow`.
The native qualification additionally checks independent closed-form expected results,
exact STATICCALL/root RETURN counts, and malformed-input/output rejection. Input is copied
once and separate output memory is allocated before the loop, including zero-call controls.
The recorded 512/2048/8192 per-call gas values are source-derived precompile costs, not
measured transaction gas or whole-node quotes. Use the same explicit `evm-gas-only` profile
and reviewed program manifest for the entire experiment; retain failures and controls.
The generator/runtime bound of 65536 exceeds the legal Osaka call count even at the
minimum 500 gas per call, and does not raise the producer's transaction gas budget.
These are finite unsaturated value-axis probes, not a global workload maximum or a tariff
change. Set `SOLC_BIN` and `NATIVE_FIXTURE_BIN` to run the real compiler/native tests.

For a finite BLS G1 MSM scalar/batch boundary probe, generate a separate catalog:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/bls_msm_values.py \
  --solc /path/to/solc --output-dir experiment/bls-msm-values
```

The 18 cases share one runtime calling precompile `0x0c`. Each input uses `K=1/4/16`
distinct points `P_i=i*G` with one shared scalar: one, `(7 << 252)-1`, alternating
`0x55` bytes, or a fixed SHA256-derived scalar. Twelve single calls cover this matrix;
three shared-random zero-call controls, K=1 dense/random four-call repetitions and one
K=16 mixed-random single call complete it. Mixed scalars use
`(SHA256("bls-g1-msm-scalar-i") & (2^255-1)) mod r` for one-based point index `i`.
Expected outputs use aggregate-scalar multiplication, checked independently against
pointwise MSM and real native execution. Curve, subgroup and fixed-width encoding tests
retain the full point/scalar boundary. Every call clears all four output words, requires
success and exactly 128 returned bytes, and compares every word. Input is copied once;
separate output memory is preallocated even for zero-call controls. Counts, K and payload
length are checked before the first call. The source-derived 12000/38256/129984 per-call
gas values include batch discounts; they are not measured transaction gas or whole-node
quotes. Retain controls and failures using the same explicit `evm-gas-only` profile and
reviewed program manifest. The 2048-call bound exceeds Osaka's legal transaction count
at the minimum 12000 gas per call; it does not raise the producer's transaction budget.
Dense binary scalars can become cheap under batch recoding, so these finite unsaturated
probes establish neither a worst-case bound nor underpricing before SP1 measurement.
Set `SOLC_BIN` and `NATIVE_FIXTURE_BIN` for the compiler/native qualification tests.

For authorization-list validation work before opcode collection, generate an offline catalog:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/authorization_values.py \
  --output-dir experiment/authorization-values
```

The twelve cases use minimal RETURN code and empty calldata: one explicit EIP-1559 reference,
one shared single-entry EIP-7702 baseline, sequential/repeated lists of 4/16/64/256 entries,
and distinct-authority lists of 4/16 entries. The count-one baseline is reused across pattern
comparisons because their signed envelopes are identical. Optional ProgramRequest
`program.authorizations` takes `{"pattern":"sequential","count":4}`; patterns are
`sequential`, `repeated` and `distinct`, with counts 1..1024. Omission retains the original
EIP-1559 construction. All authority keys are public deterministic offline fixture identities,
separate from the transaction sender; no wallets, credentials or live calls are used.
For block index `b`, sequential entries use one authority with nonces `b*K+i`, repeated lists
clone one signed entry with nonce `b`, and distinct lists use K authorities with nonce `b`.
The optional native `authorizations` report separates signed-entry counts from delegations
actually applied, verified from pre/post authority nonces and delegation code hashes. Authorities
are absent initially; subsequent witnesses include only bytecode retained from actual execution.
Canonical type-4/signature/inclusion checks and two-block nonce-progression tests gate the path.
An empty authorization list is never used as a type-4 control. Use `evm-gas-only` and the same
reviewed program/guards for comparisons; retain rejected cases and separate native receipt gas,
whole-node quotes and actual SP1 PGU. This finite cohort does not establish underpricing before
measurement, and the count bound does not increase the existing 15M transaction gas budget.
Set `NATIVE_FIXTURE_BIN` for Python native qualification and `FROZEN_NATIVE_FIXTURE_BIN` for the
Rust regression that compares default canonical bytes/reports with the prior executable.

A separate compact-signature holdout leaves that twelve-case cohort unchanged:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/authorization_holdout.py \
  --output-dir experiment/authorization-holdout
```

Its five inputs use `program.authorizations.pattern="compact"`: K=1/16/128/512 in
one block, plus K=16 in three blocks. Every entry is the same canonical authorization
with chain 167000, delegation target `0x6666666666666666666666666666666666666666`,
nonce zero and signature `(r=1,s=1,y_parity=0)`. The authority is derived by canonical
signature recovery; this pattern uses no private fixture key. The entry occupies only
30 RLP bytes. Its first entry applies once in block zero; later entries and all lists
in following blocks are stale while the authority nonce remains one. The report retains
all signed entries separately from applied delegations and unique authorities. The
three-block holdout therefore contains 48 signed entries, one applied delegation and
one authority. Pricing validation must count entries, including stale/repeated entries,
rather than payload bytes or applied/unique counts. These controls have their own
baseline and identities; the main cohort's count-one baseline is not substituted.
Native qualification checks exact type-4/RLP encoding, recovery, state progression and
minimal witness/code retention. The existing transaction gas limit still applies; a
requested count does not increase it. Set `FROZEN_AUTHORIZATION_NATIVE_FIXTURE_BIN` for
Rust byte/report compatibility checks of the original three patterns, and
`NATIVE_FIXTURE_BIN` for this holdout's Python native tests. Keep the same reviewed guest
program and resource guards for measurement; pricing adequacy requires actual results.

For access-list warming before opcode execution, generate the separate 23-case catalog:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/access_list_values.py \
  --output-dir experiment/access-list-values
```

Optional `program.access_list` takes `{"pattern":"repeated_key","count":4096}`. The six
patterns are `repeated_address`, `distinct_addresses`, `repeated_key`, `distinct_keys`,
`repeated_items` and `reversed_distinct_keys`. Counts are typed integers in 1..8192; omission
represents the empty list. Address index zero is PROGRAM; further addresses use a deterministic
initially absent fixture prefix. Storage keys are the 32-byte integers 1..K. Raw duplicates,
item grouping and key order remain intact in the signed transaction and proposal blobs. The
option also composes with existing EIP-7702 authorizations; this catalog uses EIP-1559 only.

The catalog uses empty-list references, shared one-address and one-key baselines, repeated and
unique address/key counts 16/256/4096, repeated address-plus-key items 16/256/2048, repeated
keys 2048, a reversed 4096-key control, and three-block empty/repeated-address/repeated-key
controls. Pair each three-block plan with its three-block reference. Count-one equivalent
vectors share a baseline; extra block/Anchor work is not access-list-only work.

The optional native `access_list` report contains `pattern`, `count_per_block` and one record
per block with `transaction_type`, `raw_address_items`, `raw_storage_keys`, `unique_addresses`
and `unique_storage_pairs`. For the single-user catalog these counts come from the actual
included signed transaction through canonical EIP-2718 decoding, with signer/nonce, exact-list
and inclusion gates. The multi-user CALL extension and its block/raw/union semantics are
documented below. No
accounts or storage values are preloaded for warming. Witness shape and old canonical bytes
remain unchanged when the descriptor is omitted; pricing observations must still name their
actual model and quote executable rather than claiming cross-model price identity.

The descriptor bound does not increase the existing 15M transaction gas limit. An 8192-key
request is accepted as a typed descriptor but rejected by real intrinsic-gas validation; retain
that rejection rather than raising the budget. Set `NATIVE_FIXTURE_BIN` to run all 23 Python
native qualifications and `FROZEN_ACCESS_LIST_NATIVE_FIXTURE_BIN` for omitted-option Rust byte
and stable-report-field compatibility checks. Use the same reviewed guest and unchanged quote
with the existing guarded sampler. No access-list-specific tariff or pricing-deficit claim is
introduced by this producer.

After freezing a candidate tariff, generate a separate prospective validation cohort:

```sh
~/.venv/bin/python bin/zkgas/bench/cases/access_list_holdout.py \
  --output-dir experiment/access-list-holdout
```

Its eleven measurements contain nine unfitted candidates and two explicitly labelled
replays of the main cohort's empty one/three-block references. Candidates cover repeated
addresses K=2/3; repeated address-plus-key items K=32/512/3072 and K=512 across three
blocks; repeated keys K=3072; distinct addresses K=512; and reversed distinct keys K=256.
All reuse the existing typed constructor and 15M transaction envelope. Each candidate
uses its own equal-block-count empty reference. The catalog binds both producer sources
and the shared artifact helpers, and marks previously observed references separately;
it does not claim all eleven inputs are fresh holdouts. The finite selector rejects
unlisted cases rather than labelling an already opened main case as unfitted. Set
`NATIVE_FIXTURE_BIN` to qualify all eleven natively; SP1 measurement follows independent
review and native qualification. Holdout actuals validate the frozen policy, not its fit.

### Top-level creation and repeated transactions

```sh
~/.venv/bin/python bin/zkgas/bench/cases/top_level_create.py \
  --output-dir experiment/top-level-create
```

The fixed44-request matrix uses the optional `program.transactions` descriptor and
ordinary signed EIP-1559 calls/creations. Count controls use1/16/128/512 users inside
one block, plus equal-block-count three-block references. Creation probes include
empty initcode, explicit STOP, RETURN/REVERT/INVALID, unreachable zero/JUMPDEST/PUSH32
tails through49152 bytes, returned zero-filled runtime0/32/256/24576 bytes and the
included-failure24577-byte deployment boundary. The code hash identifies each case;
initcode is signed transaction input and is not preloaded as witness runtime.

Every candidate must be included in exact order with the expected receipt status
and actual created-account/code result. Gas totals include every user receipt;
Anchor remains excluded from user opcode counts. Empty initcode is expected to run
one implicit STOP per creation, independently of having no signed code bytes. Opcode
counts in reverted initcode and completed RETURN before code-size failure are kept.
Set `NATIVE_FIXTURE_BIN` to run the full44-case native qualification test.

The explicit descriptor chooses per-tx gas limits100k/3M/8M within the existing15M
maximum, sufficient fee caps and fee-neutral funding; block/manifest budgets remain
31M/30M. A smaller declared per-tx limit avoids artificial packing rejection. These
are finite slope/boundary probes, not a claim to represent every legal envelope or
an EVM-gas-saturated maximum. Compressed one-blob overflow is a construction/transport
limit, distinguishable in the native log, not evidence of admission or pricing safety.

Use the existing `screen`/`run`/`verify`/`report` commands below with this catalog, a
frozen current host quote/model and a compatible fixed program manifest. The frozen
guest must be checked against its original source snapshot, not the new constructor
source. Rank actual proposal PGU /node-known zkGas literally; complete quotes and
same-block-count increments remain separate diagnostics. Neither changed input shape
nor new host tools requires a guest rebuild or establishes a pricing deficit.

```sh
~/.venv/bin/python bin/zkgas/bench/workload.py screen \
  --config experiment/config.json --catalog experiment/spawn-memory/catalog.json \
  --output-root experiment/run
~/.venv/bin/python bin/zkgas/bench/workload.py run \
  --config experiment/config.json --catalog experiment/spawn-memory/catalog.json \
  --output-root experiment/run
```

`screen` performs native construction and canonical quote validation. `run` performs the same
stages followed by SP1 measurement; an already screened sample is restored into a new attempt
and measured without reconstructing it. Each stage records its exact command, log, resource
outcome and artifacts. The native CLI receives `--input`, `--output`, `--output-bincode`, and
`--report`. The quote and sampler both consume `--input-bincode`; the sampler also takes `--elf`.
Native, quote and sampler hashes/lengths and32-byte public output must join before ranking.

The native constructor's public-output replay performs complete canonical proposal validation
without computing a discarded price. Witness-only and plain validation skip MPT snapshot
observation per state instance, including when host feature union enables `mpt-metrics`.
Explicit full quotes continue measuring the real MPT selectors and applying the same prices.
Host construction time is not guest workload or measured PGU.

## Measure an already retained input

No generator or compiler is required. Supply the canonical bincode and its matching native
report; the current quote validates the bincode/type and execution output before the opaque
sampler runs. The JSON GuestInput is optional for this path.

```sh
~/.venv/bin/python bin/zkgas/bench/workload.py measure \
  --config experiment/config.json --input-bincode saved/input.bin \
  --native-report saved/native.json --sample-id retained-control \
  --output-root experiment/retained-run
```

To reuse a qualified complete quote for that exact input, add `--quote-report saved/quote.jsonl`.
This option is available only with `measure`, canonical bincode and a matching native report.
The same quote validator checks exactly one complete record, sample/corpus/input hash and length,
quote executable hash, model, execution profile and native public output, including the nested
quote identities. Mismatches fail before SP1. The input must keep its original sample and corpus
identities; changing a quote key does not qualify it for another campaign.

The harness copies the original quote bytes into `quote.jsonl`, records the retained source path
and SHA256 in `started.json`/`status.json`, and writes `quote-source.json` with the source path,
hash and byte length. It creates no quote command or process outcome for retained mode: only the
subsequent SP1 invocation executes in this attempt. Without this option, quote execution, resume
behavior and failure statuses are unchanged. Existing input/output/resource guards still apply.

For direct composition, call `zkgas-sp1-estimator --input-bincode saved/input.bin --elf
programs/proposal.elf --output saved/sp1.json --resource-receipt saved/sp1-resources.json`.
For a reviewed experimental program, add `--program-manifest programs/program-manifest.json`;
the sampler rejects mismatched ELF/SDK metadata before execution and reports the manifest hash
and its declared execution profile. Omitting the manifest preserves opaque-program behavior.
The receipt option is optional for standalone execution; forced termination without a supervisor
can retain SDK shared memory. The backend treats bytes as opaque; direct callers
must perform canonical quote validation and the identity/output join themselves.

## Verify, resume and report

```sh
~/.venv/bin/python bin/zkgas/bench/workload.py verify --output-root experiment/run
~/.venv/bin/python bin/zkgas/bench/workload.py report \
  --output-root experiment/run --output experiment/summary.json
```

Repeating `run`, `screen` or `measure` validates the frozen contract, successful artifacts,
gzip contents, input/output joins and recomputed metrics before skipping a completed sample.
Failed/partial attempts require explicit `--retry-failed` and always produce fresh attempt
directories. Corrupt successful attempts require investigation and cannot be bypassed with
that flag. `verify` fails on incomplete or corrupt evidence. `report` retains those outcomes
as unscored rows and includes catalog cases never attempted after fail-fast termination.
Reports and all attempt files use exclusive creation; choose a new summary filename to rerun.

`screen` and `run` stop at the first retained failure by default. Add
`--continue-after-failure` to attempt the remaining finite catalog while keeping every failure;
the command still exits nonzero if any attempt failed. This option never retries or skips a
previous failed attempt without validating it. On resume it validates retained failure artifacts,
source and contract, reports `retained_failure` with the original status and attempt, and continues
to later cases without creating another attempt. The overall exit remains nonzero. Creating a
fresh attempt still requires `--retry-failed`. Corrupt successful artifacts, unvalidated partial
attempts and source/contract changes remain fatal even with continuation enabled.

Successful attempts retain gzip inputs only after a verified roundtrip. A single input is
restored at a time. Failed stages preserve partial bytes and logs. Cancellation by SIGINT,
SIGTERM or SIGHUP terminates owned descendants, including detached children, and retains the
attempt. A surviving child after leader exit is a failure. Do not use the supervisor in a
process with unrelated child workloads.

`native_rejected` means construction/execution/count screening failed; consult its native log
and receipts, including out-of-gas outcomes. `quote_failed` means canonical/native pricing
validation failed. `measurement_failed` means the SP1 process failed. `guard_failed` identifies
resource/process cleanup limits. `validation_failed` identifies an identity, model, policy or
output mismatch. None receives an actual PGU score. `screened` is accepted but unmeasured.

The summary sorts exact rational actual/node values using integer fractions and preserves
the full-quote ratio separately. It embeds standard observation rows consumable through the
existing zkGas comparison contract. Paired deltas must be interpreted with the frozen catalog
envelope and controls; they do not replace the literal ratio objective. Atlas exports should
contain only fixed inputs, original measurement/quote/native evidence and portable manifests;
exclude runner code, executable files, private operator configuration and host paths.

## Top-level creation deployment followups

`top_level_create_followup.py` produces 38 fixed model-development cases: deployed-byte
boundaries through 24576, repeated deployment of identical code within/across blocks, three
runtime contents generated by compact MCOPY sequences, and ordinary CALL-return controls
including 24577 bytes. The six content cases keep identical opcode paths for each length;
only the one-byte seed changes. This avoids embedding the returned runtime in the signed
input and obscuring deployment cost with payload pricing.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/top_level_create_followup.py \
  --output-dir experiment/top-create-followup
NATIVE_FIXTURE_BIN=target/debug/zkgas-fixture ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_top_level_create_followup.py
```

The catalog labels three byte-identical known reference replays from the original 44-case
matrix and 35 new development cases. These are calibration followups, not prospective
holdouts. Baselines match transaction kind, block count and declared gas; explicit
`repeat_reference_sample_id` links retain same-code count comparisons separately. Every
request uses the existing ProgramRequest constructor, with successful receipt, all-user
opcode, nonce and actual created-state checks. No guest, native or tariff change is needed.
Measure whole-proposal PGU with the existing frozen-profile workflow; report literal
actual/node ratios, complete quotes and matched increments separately.

## Existing runtime loading controls

`code_loading.py` produces ten ordinary CALL cases: a one-byte STOP reference, plus 32,
4096 and 24576 bytes of initial account runtime with zero, JUMPDEST or PUSH32-pattern
suffixes. Every runtime starts with STOP, so all suffix bytes remain unexecuted. Code is
stored in the initial account/witness; transaction calldata stays empty. The catalog
links each candidate to the one-byte reference and each nonzero pattern to its same-length
zero reference, retaining both positive and negative content differences.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/code_loading.py \
  --output-dir experiment/code-loading
NATIVE_FIXTURE_BIN=target/debug/zkgas-fixture \
ZKGAS_QUOTE_BIN=target/debug/zkgas-quote ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_code_loading.py
```

Qualification checks actual code-byte/code-hash counters in the frozen quote, separately
from canonical transaction payload counters. The one-byte reference shares its STOP code
hash with the existing checkpoint account; larger runtimes add a distinct hash, so unique
code-byte deltas include their full length. This accounting detail is retained rather than
assuming a length-minus-one delta. These are model-development workload observations,
including one known reference replay, with whole-proposal actual PGU measured separately;
they do not establish deployment-only costs or proving capacity.


## Prospective code-preparation validation

```sh
~/.venv/bin/python bin/zkgas/bench/cases/code_preparation_holdout.py \
  --output-dir experiment/code-preparation-holdout
```

This16-row set was selected after freezing the64 unique-byte and8192/nonempty-event
plus32/prepared-byte candidate policy. Fourteen inputs are prospective and two are
explicit known reference replays. It covers fresh loaded-code lengths/content,
deployment size/repetition, MCOPY-generated code and an internal CREATE factory
whose outer transaction either STOPs or REVERTs. The factory checks CREATE success
before taking that terminal path. Successful child preparation must still be
counted when the ancestor reverts.

Each catalog row declares its expected `code_preparation` event/byte selectors.
A self baseline explicitly means an absolute validation point without a matched
gas/count control; do not manufacture isolated marginal claims. Ordinary code
loading contributes no deployment-preparation events. Use the same canonical
constructor and frozen program; native qualification can use the unchanged creator.
SP1 actuals may be collected independently from pricing, but candidate precision
requires actual new-model native quotes on those same canonical inputs. Do not
refit from prospective results. These finite observations make no universal bound,
real-proof, GPU-throughput or activation claim.

## Repeated initcode preparation

`initcode_preparation.py` generates 24 fixed cases against frozen code-preparation
model `a2988920…`. A small factory builds its child initcode in memory using MCOPY,
then loops over successful CREATE or CREATE2 frames. The parent runtime does not
expand with repetition count. Every child address is checked; CREATE2 uses distinct
consecutive salts. Children STOP or return one zero byte, followed by unreachable
zero, JUMPDEST or PUSH32-pattern tails. All requests use one block, one ordinary CALL,
empty calldata and the existing 15M user gas envelope.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/initcode_preparation.py \
  --output-dir experiment/initcode-preparation
NATIVE_FIXTURE_BIN=target/debug/zkgas-fixture \
ZKGAS_QUOTE_BIN=target/debug/zkgas-quote ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_initcode_preparation.py
```

The core matrix combines CREATE/CREATE2, 4096/49152-byte initcode and 1/16/128
repetitions. Controls retain zero repetitions, tail content, ancestor REVERT,
one-byte output and empty initcode. Native qualification checks completed child
and parent opcode counts, real receipt status, factory nonce and preparation
events/bytes. Included ancestor REVERT rolls back accounts and factory nonce while
retaining spent preparation work. Empty-output cases have no nonempty-output
preparation events; one-byte cases have one event and one byte per child.

References distinguish repetitions, length, content, rollback and output changes.
Only the 49152-byte JUMPDEST cases have a matching zero-repetition image control;
short cases use their same-length N1 reference. Long/short differences-in-differences
use each length's own repetition reference. Content/output changes can also alter
parent memory work and CREATE2 trie addresses. Existing CREATE/CREATE2 wrapper
prices remain active: these model-development cases do not presuppose underpricing.
Measure whole-proposal actual PGU/node-known zkGas, full quote closure and matched
increments separately with the existing bounded harness. No Rust, guest or tariff
change is needed; this finite set is not a gas-saturated capacity measurement.

## Minimal DELEGATECALL repetition

`delegatecall_repetition.py` produces 16 fixed cases: child STOP, RETURN(0,0),
REVERT(0,0) or INVALID, each repeated 0, 1, 1024 or 32768 times. A small parent
creates one tiny child even at N0, keeps its address and loops with fixed-width
counter immediates. Every DELEGATECALL forwards exactly 64 gas with zero input and
output lengths. The parent requires the expected per-call status and zero return
data, then ends in STOP. The child account is created during execution; no extra
child code or account is added to prestate. An identical code byte string may
already serve an existing system account, so code-hash uniqueness is not assumed.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/delegatecall_repetition.py \
  --output-dir experiment/delegatecall-repetition
NATIVE_FIXTURE_BIN=target/debug/zkgas-fixture \
ZKGAS_QUOTE_BIN=target/debug/zkgas-quote ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_delegatecall_repetition.py
```

All requests retain the existing one-block, one-CALL, empty-calldata, 15M envelope
and explicit `evm-gas-only` profile. Completed opcode maps include the once-only
child initcode RETURN and the repeated child terminals. INVALID is exceptional
and attempted, never a completed opcode. Native qualification checks both maps,
exceptional counts, real user receipt, factory nonce1 and actual preparation
selectors. A failed status cannot substitute OOG for the expected completed
REVERT terminal. Wrong status or nonempty return data takes an unreachable failure
path instead of silently shortening the loop.

STOP/RETURN are the minimal confirmed-wrapper candidates. REVERT/INVALID variants
are explicitly labeled high-price failure controls: existing normal-fallback or
feature-error pricing already applies a 5M/event floor. Only within-terminal
repetition comparisons match runtime/setup; each series retains its N0 reference
and a separate N1 repetition reference. Use frozen model `a2988920…` and the
existing native, quote and bounded SP1 workflow. Literal whole-proposal actual/node
ranking, full quote closure and matched increments remain separate. No Rust,
guest or tariff change is involved, and no actual-cost conclusion follows from
the opcode's wrapper price alone.

The [measured checkpoint](../../../docs/research/2026-10-30-delegatecall-repetition.md)
records all 16 actual observations and separate native-only controls. Existing
pricing covered all full quotes and all 20 matched repetition increments;
REVERT/INVALID retain their deliberately conservative failure floors.

## DELEGATECALL density profile

The same producer accepts `--profile density` for eight STOP/RETURN(0,0) cases
with 0, 64, 1024 or 32768 total calls. It retains a fixed 64-call body even at
N0 and selects 0, 1, 16 or 512 batches using a fixed-width immediate. Each call
forwards 64 gas with zero input/output lengths, then POPs its status. The real
one-time CREATE and its address guard remain; per-call status and return-data
guards are absent. This isolates a denser call/child workload from conservatively
priced parent checks. It does not establish underpricing before measurement.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/delegatecall_repetition.py \
  --profile density --output-dir experiment/delegatecall-density
NATIVE_FIXTURE_BIN=target/debug/zkgas-fixture \
ZKGAS_QUOTE_BIN=target/debug/zkgas-quote ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_delegatecall_density.py
```

`make_case(..., profile='density')` and `prepare(..., profile='density')` expose
the same construction. The default `checked` profile preserves all 16 original
request bytes; tests bind their frozen request hashes and can also compare the
original files when `CHECKED_DELEGATECALL_CASES` points at that catalog directory.
Density uses distinct IDs, N0 baselines and N64 repetition references. Its default
catalog uses 0/64/1024/32768 calls; the API accepts multiples of 64 through 65536.
Failed child terminals are not density cases.

A successful outer receipt alone is insufficient for this guardless profile.
Qualification checks exact child terminals/call counts, zero exceptional
instructions, the deployed child/runtime, factory nonce1 and the current quote.
Short batches, an empty target, parent OOG and zero-forwarded-gas RETURN must be
rejected by those gates; zero-gas STOP is valid. No additional child witness
padding, Rust/guest build or pricing change is needed. Keep literal whole-proposal
actual/node ratios, full quote closure and within-terminal N0/N64 increments
separate; this finite density test does not measure saturation or proving speed.

`--profile density --zero-arguments push0` generates six prospective validation
programs: STOP/empty RETURN at 0, 4096 or 65536 calls. Only the four call-argument
zeros change to PUSH0; the 64-call body, real deployment, gas forwarding and
native completion gates remain. Density accepts up to 65536 calls; checked
keeps its original 32768 bound. Default checked and PUSH1 density requests
remain byte-identical to the retained 16- and 8-case cohorts.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/delegatecall_repetition.py \
  --profile density --zero-arguments push0 --model-id "$ZKGAS_MODEL_ID" \
  --output-dir experiment/delegatecall-push0
```

Set `ZKGAS_MODEL_ID` to the content address of the frozen candidate before
opening these actual-workload observations. `--model-id` records dataset
metadata; prices come from the selected quote executable, and GuestInput
construction does not depend on this value. Record native inclusion, exact
child terminals and gas headroom before sampling. These prospective rows must
not be used to choose the candidate's tariff or mixed with its fitting cohort.

## BLOCKHASH density

Generate twelve programs querying the nearest valid ancestor (`B-1`), farthest
valid ancestor (`B-256`) or expired height (`B-257`). Each performs one semantic
warmup query, then 0, 64, 4096 or 262144 bulk queries using a fixed 64-site
`DUP2 / BLOCKHASH / POP` body. **Total user-program BLOCKHASH events are bulk count plus one**,
including N0, which retains the same unused body. The full-block quote also
includes Anchor events; this user-program count excludes them. The target and batch-count
immediates have fixed widths. There is one user CALL, empty calldata and a 15M
EVM gas budget; no child deployment or extra witness padding is needed.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/blockhash_density.py \
  --output-dir experiment/blockhash-density
NATIVE_FIXTURE_BIN=target/debug/zkgas-fixture \
ZKGAS_QUOTE_BIN=target/debug/zkgas-quote ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_blockhash_density.py
```

`make_case(mode, bulk_queries)` also exposes `current` (`B`) and `future`
(`B+1`) as native-only boundary controls, outside the twelve-case catalog.
Counts must be multiples of 64 through 262144. Native qualification checks the
contiguous ancestor headers, semantic warmup, exact completed opcode counts,
zero exceptional instructions and successful user receipt. Short execution,
an incorrect semantic target and parent OOG must fail qualification.

The default catalog records frozen model `2fb4bb71…`. Optional `--model-id`
changes catalog metadata only; the selected quote executable supplies prices.
Use its actual model identity when selecting another frozen quote executable.
Within each mode, N0 and N64 references separate absolute workload from
repetition. Cross-mode differences include semantic guards and target arithmetic,
so they are not isolated lookup costs. Retain literal whole-proposal actual
SP1 PGU/node-known zkGas, complete quote closure and matched increments as
separate diagnostics. No pricing, capacity or throughput conclusion precedes
actual measurements.

For a separately frozen tariff validation, `--profile prospective` emits six
new inputs: farthest/expired heights with 0, 8192 and 196608 bulk queries.
Each runtime executes an additional `PUSH0 / POP` prelude, including N0;
all jump destinations and expected counts are regenerated. These inputs use
distinct `-push0` IDs and are marked as prospective validation. The caller
must freeze its candidate price before measuring them; the label alone does
not establish that chronology. Pass `--model-id` for the candidate executable.
The within-mode repetition reference is N8192 in this profile.

The BLOCKHASH tests also retain a regression for the historical reversed operand
order. `NUMBER / PUSH offset / SUB` computes `(offset - NUMBER) mod 2^256`,
which is a huge future height in these fixtures. The maintained producer uses
`PUSH offset / NUMBER / SUB` to query `NUMBER - offset`. Tests mutate only a
copy of the generated bytecode and preserve its length and jump destinations.
Nearest/farthest variants with the historical order must reach the semantic
INVALID guard before bulk execution; the optional native lane rejects them
without publishing canonical input or report files. For current/expired modes,
both targets can return zero and even have identical completed opcode totals.
The independent interpreter therefore records actual BLOCKHASH operands and
compares them against the intended height; receipt success or a zero result
alone does not establish the requested boundary. No old calibration artifact or
legacy launcher is changed by this regression.

```sh
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_blockhash_density.py
NATIVE_FIXTURE_BIN="$FROZEN_NATIVE" ZKGAS_QUOTE_BIN="$FROZEN_QUOTE" \
ZKGAS_EXPECTED_MODEL_ID="$ZKGAS_MODEL_ID" ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_blockhash_density.py
```

Set the three variables to the retained native constructor, quote executable and
its compiled model identity. The tests require no guest build, Cargo build or
new SP1 measurement. The original twelve density and six prospective canonical
requests remain unchanged.

## Arithmetic density

`arithmetic_density.py` emits 24 ordinary ADD/MUL cases: four operand classes per
operation, each with 0, 4096 or 262144 bulk operations. ADD uses zero `(0,0)`,
small `(7,11)`, all-limb carry `(2^256-1,1)` and deterministic asymmetric full-width
mixed operands. MUL uses the same zero/small/mixed pairs and the overflowing wide
pair `(2^256-1,2^256-3)`, whose uint256 result is 3. Mixed values come from SHA256
of `zkgas-arithmetic-operand-a-v1` and `zkgas-arithmetic-operand-b-v1`, with bit255
set; the catalog records both exact operands and the modulo-2^256 expected result.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/arithmetic_density.py \
  --output-dir experiment/arithmetic-density
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_arithmetic_density.py
NATIVE_FIXTURE_BIN="$FROZEN_NATIVE" ZKGAS_QUOTE_BIN="$FROZEN_QUOTE" \
ZKGAS_EXPECTED_MODEL_ID="$ZKGAS_MODEL_ID" ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_arithmetic_density.py
```

The default frozen model is `bbce9be0…`; optional `--model-id` changes catalog
metadata only. `make_case(operation, operand_class, bulk_operations)` accepts ADD
classes `zero/small/carry/mixed256` and MUL classes `zero/small/wide/mixed256`.
Counts must be multiples of 64 through 262144. Every operand and expected-result
immediate uses PUSH32, and the counter uses fixed PUSH widths. N0 retains the
same unused 64-site body. There is one user CALL, empty calldata, one block and
15M user gas, with the explicit `evm-gas-only` execution profile.

The initial semantic operation and every bulk operation consume the same ordered
top-first pair `(a,b)`. The retained stack is `[b,a,index]`; `DUP3/DUP3/op/POP`
leaves both original operands unchanged. The target pair executes **N+1 times**,
including the semantic guard. The loop also executes **N/64 counter ADDs** with
a different pair `(1,index)`, so ADD's completed opcode total is `N+1+N/64`;
MUL's completed opcode total is `N+1`. Independent tests decode actual consumed
pairs at every target PC and distinguish the counter. Swapping asymmetric
operands can preserve all results, guards and opcode totals; ordered-pair
qualification must still detect it. Wrong-result guards, short execution,
corrupted DUP indexing and parent OOG have separate regression coverage.

For each accepted case, retain three quantities: the complete proposal estimate,
the node-observable estimate for **each block and their sum**, and actual SP1 PGU
for that same proposal. Report proposal underestimation and node-sum
underestimation independently, including N0 and small controls in literal
actual/node ranking. Complete quote coverage cannot establish mining admission
coverage. Same-operation, same-operand N0/N4096 increments help separate fixed
from scalable gaps; they do not replace either underestimation judgment or assign
actual PGU to individual opcodes. Each case currently has one block, so its node
sum equals that block's estimate. Proposal-aware allocation into mining remains
a future question and is not implemented here. This finite matrix does not
claim a maximum workload, throughput, universal bound or production activation.
Use the existing guarded workload tools to retain raw results and failures; no
Rust/guest build, new SP1 sampling or pricing change is part of these unit tests.

### Paired DUP/POP controls

Use `--profile dup-pop` to generate twelve diagnostic controls: ADD/MUL warmup
families, zero/mixed256 operand pairs and 0/4096/262144 bulk sites. The only
bulk bytecode substitution is `DUP3/DUP3/op/POP` → `DUP3/DUP3/POP/POP`.
The four-byte site, original operand copies, loop, immediate widths, initial
arithmetic-result guard and outer STOP remain. Every control still performs
one real arithmetic operation in its guard; counter ADDs remain N/64. It has
N bulk pair-copy events, zero bulk arithmetic events and 2N bulk POPs. Tests
record the actual copied values by PC: the first POP consumes `a`, the second
consumes `b`; both retained operands survive unchanged.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/arithmetic_density.py \
  --profile dup-pop --output-dir experiment/arithmetic-dup-pop
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_arithmetic_density.py
```

`make_case(..., profile='dup-pop')` accepts only the zero/mixed256 classes;
the default `arithmetic` profile preserves all 24 original request bytes and
case records. The catalog's generator SHA changes when its source changes;
the frozen original catalog and its source revision are retained as explicit
pair identities, alongside each original arithmetic request SHA. No old
catalog, run or raw observation is rewritten.

Each control has its own N0 and N4096 references. Its unused body differs from
the arithmetic program even at N0, so identical executed opcode totals do not
justify reusing the original canonical input or actual PGU. Compare each
profile's increment from its own baseline before comparing the two increments.
This separates a composite arithmetic-versus-POP contribution from much of the
shared copying and loop work; POP has a real cost, and changed code content can
affect setup. It does not isolate the actual cost of ADD or MUL. Zero/mixed
controls can also reveal operand-dependent copying behavior. Preserve the same
three quantities and both absolute underestimation judgments above; explanatory
matched increments must not erase controls or node-sum deficits. No tariff is
changed by constructing these paired inputs.

## Balanced stack density

`stack_density.py` emits 24 cases: `push0-pop`, `dup1-pop`, `dup3-pop`, `dup8-pop`,
`dup16-pop`, `swap1-swap1`, `swap8-swap8` and `swap16-swap16`, each with
0/4096/262144 executed pairs. A fixed 64-pair body remains in the runtime even
at N0, with fixed-width batch-count immediates. Every pair is two bytes and
returns to the same 17-word stack. The words are distinct deterministic full-width
values: SHA256 of `zkgas-stack-seed-v1-{i}` with bit255 set, for i0..16 in
bottom-to-top order. The catalog records their exact hex values.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/stack_density.py \
  --output-dir experiment/stack-density
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_stack_density.py
NATIVE_FIXTURE_BIN="$FROZEN_NATIVE" ZKGAS_QUOTE_BIN="$FROZEN_QUOTE" \
ZKGAS_EXPECTED_MODEL_ID="$ZKGAS_MODEL_ID" ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_stack_density.py
```

`make_case(pattern, bulk_pairs)` accepts multiples of 64 through 262144. The
counter occupies memory word0 and leaves the stack before each body executes:
DUP1 sees seed16, DUP16 sees seed1, and SWAP16 exchanges seed16 with seed0 before
its second SWAP restores them. There is no changing counter on top of the seed
stack. For B=N/64 batches, counter work includes B ADDs, 2B+1 MLOADs and B+1
MSTOREs; only 32 bytes of memory are used. At the end, 17 explicit PUSH32/EQ
semantic guards check and consume every original word. Independent tests decode
actual stack values, accessed depths, ordered swaps, byte-addressed memory,
final counter and complete opcode counts. A wrong-depth balanced DUP may pass
ending guards; its accessed-value oracle must still detect the mismatch.

These are one-block, one-CALL, empty-calldata inputs with 15M gas and the explicit
`evm-gas-only` profile. The frozen quote model is `bbce9be0…`; `--model-id` records
metadata only. Wrong ending values, short execution, a broken SWAP pair and
parent OOG have separate negative coverage. Existing arithmetic and DUP/POP
cohorts remain unchanged; no guest, binary, dependency or price is updated.

Retain the complete proposal estimate, each node-observable block estimate and
their sum, and actual SP1 PGU for the same proposal. Report proposal and node-sum
underestimation independently, including N0 in literal actual/node ranking.
Same-pattern N0/N4096 increments help explain fixed versus scalable gaps; they
cannot replace those absolute judgments. Balanced pairs and their memory-counter
loop are composite observations and cannot uniquely identify isolated POP, DUP
or SWAP tariffs. This finite matrix makes no maximum-workload, throughput,
universal-bound or production-admission claim. Actual measurements use the
existing guarded workload tools after independent native qualification.

## Prospective normal-dispatch reserve validation

`normal_dispatch_holdout.py` creates 24 holdout-only cases after the 64-PGU reserve
was frozen in model `c8936124…`: `dup16-pop`, `swap16-swap16`, `xor`, `shl255`,
`sar255`, `callvalue-pop`, `timestamp-pop` and `number-pop`, each at
0/8192/196608 balanced body operations. A fixed 128-site body remains present at
N0. Batch-count pushes have fixed widths; the counter stays in memory word0,
leaving the same 17 full-width seed words on the body stack. All 17 ending words
are checked. No earlier development producer or input is changed.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/normal_dispatch_holdout.py \
  --output-dir experiment/normal-dispatch-holdout
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_normal_dispatch_holdout.py
NATIVE_FIXTURE_BIN="$FROZEN_NATIVE" ZKGAS_QUOTE_BIN="$CANDIDATE_QUOTE" \
ZKGAS_EXPECTED_MODEL_ID="$ZKGAS_MODEL_ID" ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_normal_dispatch_holdout.py
```

`make_case(pattern, bulk_operations)` accepts multiples of 128 through 196608.
XOR copies seed15 then seed16, so its actual top-first ordered operands are
seed16 and seed15. SHL/SAR copy negative seed16 and shift by 255; SHL yields
bit255 and signed SAR yields all ones. Arithmetic, shift and contextual cases
execute one checked semantic warmup in addition to their bulk target events.
DUP16 and double-SWAP16 have no extra warmup. Native qualification independently
checks access depths, operand order, signed results, the final counter, complete
opcode maps and successful receipts. Wrong balanced copies can preserve the
ending seeds, so receipt success alone is insufficient.

Context warmups compare exact declared values: CALLVALUE=0, NUMBER=1166000 and
TIMESTAMP=1786021201 for the frozen one-block constructor. Qualification must
independently match these with the canonical signed transaction and actual
witness header; metadata alone is not an oracle. Inputs are one CALL, empty
calldata and 15M gas under `evm-gas-only`, with no CREATE or code preparation.
`--model-id` labels metadata only and does not select compiled prices.

Retain three primary quantities per sample: complete proposal estimate,
node-observable estimate for each block plus their sum, and actual SP1 PGU for
that same proposal. Report underpricing of each estimate independently,
including every N0 control. Same-pattern N0/N8192 increments explain scalable
gaps; they do not replace literal comparisons or identify isolated opcode
costs. These holdouts must not silently refit the frozen reserve. No price,
guest, capacity or proposal-to-mining allocation change is made here.

## Proposal size controls for ordinary opcode composition

`block_scaling.py` keeps the prospective DUP16/POP and paired SWAP16 runtime bytes
unchanged while placing one identical user CALL in each of 1, 4, 16 or 32 blocks.
Each pattern has N0 and N196608, yielding16 inputs. The four b1 inputs are exact
known references from the normal-dispatch validation panel; reuse their retained
actuals, rather than describing them as new holdouts. The twelve multi-block inputs
need independent native qualification and separate actual measurements.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/block_scaling.py \
  --output-dir experiment/block-scaling --model-id "$ZKGAS_MODEL_ID"
NATIVE_FIXTURE_BIN="$FROZEN_NATIVE" ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_block_scaling.py
```

Set the model metadata to the frozen candidate64's content address; prices still
come from the selected executable. ProgramRequest already supports these block
counts. The producer multiplies the declared aggregate opcode counts, preserves
per-transaction EVM gas and code, and validates block bounds. Context-sensitive
NUMBER/TIMESTAMP guard programs are deliberately excluded because their expected
values would change across blocks. No new native or guest build is required.

Keep the complete proposal estimate, each block's node estimate and literal sum,
aggregate-ceil diagnostic, and actual identical-proposal PGU. Compare N0/high work
at the same block count and compare block-count growth separately. Do not divide
actual proposal PGU into invented block actuals or allocate the proving-only quote
to mining. Retain guard failures/native rejects rather than silently lowering the
requested block count. The bounded sampler's existing6GiB guard still applies;
this panel studies fixed versus repeatable gaps, not saturation or proving speed.

## Storage/hash composition and caught rollback

`storage_hash_composition.py` emits 45 new validation requests under frozen model
`c8936124…`, using one ordinary program account with initially empty storage.
There is no custom `storage_state`, padding or precreated child. Each user checks
slot2=0, seeds5, runs a guarded kernel5→7, and finally restores5 then clears0.
The three fixed64-site kernels perform dirty writes11→7, hash32 zero bytes, or
combine11→hash→7. Bulk writes stay nonoriginal and nonzero; they do not repeatedly
restore the transaction's original zero. A once-only Ethereum Keccak guard also
runs for dirty/N0. SHA3-256 is a different function and is not substituted.

Layouts are `direct`, one self-DELEGATECALL returning32 bytes, and one caught
self-DELEGATECALL REVERT with the same32-byte marker. All outer users succeed.
The marker is derived from the completed loop counter, not just the declared N.
Parent checks child status, exact return length/data, slot2 restored to5 and then cleared
to0. Parent's initial SLOAD already warms slot2, and the self target is warm:
this panel covers warm dirty journal work and rollback, not cold-first-access
warm-set rollback. The latter remains a separate test dimension.

```sh
~/.venv/bin/python bin/zkgas/bench/cases/storage_hash_composition.py \
  --output-dir experiment/storage-hash-composition
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_storage_hash_composition.py
NATIVE_FIXTURE_BIN="$FROZEN_NATIVE" ZKGAS_QUOTE_BIN="$FROZEN_QUOTE" \
  ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_storage_hash_composition.py
```

`make_case(kernel, layout, rounds, transactions, blocks)` accepts rounds0..32768
in multiples of64, transactions1..16 and blocks1..4. It preserves15M per-user gas,
31M constructor blocks and12M child forwarding; it never adjusts gas or silently
prunes combinations. A domain-valid combination can still fail native admission
or resource guards and must retain that failure without a PGU value.

The matrix contains27 one-user/one-block inputs (three kernels × three layouts ×
N0/2048/32768), twelve mixed-kernel packing inputs (four users × N0/8192 or sixteen
users × N0/2048, across all layouts), and six mixed four-block inputs (one user ×
N0/32768). High packing cases preserve32768 total rounds per block. Exact opcode
counts aggregate every user and block; kernel/guard metadata states its per-user
scope. Counter memory64, marker128, parent input tag256 and hash memory0..31 are
disjoint; child memory is separate. Code retains the same unused body at N0.

Independent concrete tests inspect actual storage keys/values/originals, hashed
bytes, all counters/markers, child input/output, rollback and every completed
opcode. The optional native lane runs three small positives plus focused
negatives, not all45 qualifications. It deliberately shows that a wrong slot in
a reverting child can pass native receipt/count gates while failing the operand
oracle; native counts alone are insufficient. Complete45 native qualification
and actual SP1 sampling are separate evidence gates.

Keep node estimates per block plus their literal sum and aggregate ceiling,
complete proposal estimate, and actual SP1 PGU for the identical proposal. Retain
both underpricing flags, shortfalls, controls and failures. Own-N0 references
match layout, user count and block count; cross-kernel/frame comparisons remain
composition diagnostics because code sizes differ. No per-block actual,
isolated opcode cost, price refit, capacity or proposal-budget allocation is
claimed. `--model-id` changes metadata only; no old producer, Rust, guest, SDK or
pricing source is modified.

## Minimal-multiproof storage-access growth

`storage_access_growth.py` creates 26 new validation inputs under frozen model
`c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef`:
warm/present/absent access × 8192/16384 initialized slots × 0/64/1024/4096 bulk reads,
plus warm 65536 for each state size. Each input uses one block, one default EIP-1559
user CALL with the constructor's 15M EVM gas limit, empty calldata and explicit
`evm-gas-only` execution at screening/measurement. The producer omits `transactions`
because the existing canonical builder prohibits combining it with `storage_state`.

```bash
~/.venv/bin/python bin/zkgas/bench/cases/storage_access_growth.py \
  --output-dir /tmp/storage-access-growth-cases
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_storage_access_growth.py
# Optional: select already frozen native and quote binaries, without rebuilding.
NATIVE_FIXTURE_BIN="$NATIVE_BIN" ZKGAS_QUOTE_BIN="$QUOTE_BIN" \
  ~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p test_storage_access_growth.py
```

All raw logical slots 0 through `state_size-1` contain 1. Warmup reads slot 0
and checks 1 without adding it to the checksum. The fixed 64-site loop reads
`base + stride*i`: warm repeats 0, present visits 1 through N, absent visits
1048577 through 1048576+N. Counter and checksum occupy separate memory words;
final guards require i=N and checksum=N for warm/present or 0 for absent. No
storage writes occur. `proof_slots` is exactly the deduplicated union of warmup
and actual bulk keys; insert/delete targets are empty. The canonical builder
constructs only the target multiproof from real background state, without witness
padding or arbitrary hashed-key injection.

The tests independently execute every concrete operand, value and cold/warm
access in all 26 programs and check their exact proof targets and opcode maps.
Checksum-preserving wrong-base and wrong-stride programs fail the operand oracle.
Optional native probes authenticate the storage root and each proof target using
an independent Ethereum Keccak/RLP/MPT oracle, check node observed unique storage
resources and reject missing proof targets and wrong warmup values. A deliberately
wrong-base probe can pass native receipts, checksum and opcode counts while the
operand oracle still rejects it.

Retain all six own-N0 controls, every failure, complete proposal estimates, node
per-block estimates and their literal sum, actual identical-proposal SP1 PGU and
both deficit flags. Compare growth within each mode/state-size series and matched
state-size references. The witness trie is reconstructed once; these inputs do
not imply repeated materialization, a universal depth/workload bound, per-block
actual attribution, refitted prices or capacity evidence. Existing output
directories are rejected; model ID is catalog metadata and does not alter requests.

## Literal block sums in workload ranking

The workload harness ranks `actual_sp1_pgu / node_known_pgu` using the **literal sum
of each block's rounded node estimate** (`sum_of_block_known_pgu`). It validates
each `known_subtotal_pgu` against its scaled subtotal, verifies the sum of scaled
block subtotals, and checks both the literal sum and aggregate ceiling from the
quote. Missing block or literal-sum evidence rejects a new run before SP1.

Metrics also retain `node_aggregate_ceil_pgu` (one ceiling after summing scaled
subtotals) and ordered `node_per_block_pgu`. These differ legitimately: two blocks
priced at1.1PGU each have per-block ceilings `[2, 2]`, literal sum4 and aggregate
ceiling3. The actual remains a whole-proposal measurement, with no per-block actual
attribution.

Previously retained attempts with the exact original six-field metric shape remain
readable after their stored aggregate-ceiling values and all raw evidence validate.
Reports recompute canonical literal-sum metrics from the validated quote and actual,
and expose the original values as `stored_metrics` with
`stored_metrics_shape: legacy_aggregate_ceil`. Reporting never rewrites retained
status, quote or SP1 files. Compatibility applies only to reading old stored
metrics; it does not permit missing canonical quote fields in new runs.

## Blocks near30M charged gas

### Near30M charged-gas density composition

`gas_budget_density.py` generates20 fresh one-block requests under frozen model
`c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef`:
JUMPDEST, PUSH0/POP, DUP16/POP, paired SWAP16 and mixed JUMPDEST/DUP16/POP;
4or16 CALL users; each packing/pattern keeps its ownN0/full pair.

```bash
~/.venv/bin/python -S bin/zkgas/bench/cases/gas_budget_density.py \
  --output-dir experiment/gas-budget-density-cases
~/.venv/bin/python -S -m unittest discover \
  -s bin/zkgas/bench/tests -p test_gas_budget_density.py -v
NATIVE_FIXTURE_BIN=/path/to/frozen/native \
ZKGAS_QUOTE_BIN=/path/to/frozen/quote \
~/.venv/bin/python -S -m unittest discover \
  -s bin/zkgas/bench/tests -p test_gas_budget_density.py -v
```

The2048-site body is unchanged betweenN0/full. Its memory counter is separate
from17 distinct full-width stack seeds. Runtime guards check the exact ending
counter and every seed. Mandatory stdlib tests decode the entire fixed body,
check exact opcode maps and closed-form gas, and concretely verify bounded
iterations with actual ordered operand oracles and count-preserving negatives.
Full loops are qualified using successful native receipts and exact opcode
counts; unit tests do not interpret tens of millions of instructions in Python.
The opt-in native smoke checks three representative cases and one semantic
rejection; independent qualification must still cover all20 canonical inputs.

Charged gas is `21000 + 489 + 3 + batches*(55 + 2048*body_site_gas)` per user:
empty-calldata intrinsic, fixed instruction gas, one-word memory expansion,
then loop plus body. No refund applies. The full batch count is the largest
integer at or below29,990,000 whole-block gas including the frozen constructor's
167355-gas Anchor. This Anchor value is fixture-specific and qualification
compares the actual first receipt. Full predictions are29,831,387..29,989,743;
actual qualification requires29,700,000..30,000,000. Per-user limits use the
full-selected gross requirement plus100000gas, and are identical in eachN0/full
pair. They fit15M per-user and the final user's remaining31M native-header gas.

Keep complete proposal quote, every block's node-observable quote and literal
sum, and actual identical-proposal SP1PGU. An actual-minus-node gap above2B PGU
within30M charged block gas is high priority. OwnN0 and both absolute deficits
remain visible; these prospective actuals never fit coefficients. SDK6.3,
ELFb76, pricing and earlier producer bytes remain unchanged. These finite
composition cases do not establish isolated opcode prices or a global bound.

### Gas-budget journal composition

`gas_budget_journal.py` creates12 unfitted inputs: dirty/direct,
dirty/caught-REVERT and dirty+hash/caught-REVERT, each at4/16 successful user CALLs
with matched zero/full controls. Each full input selects the largest integer
number of64-round batches below29,990,000 charged block gas, including the
fixture's167355-gas Anchor. The full qualification range is29.7M..30M charged gas.

```bash
~/.venv/bin/python -S bin/zkgas/bench/cases/gas_budget_journal.py \
  --output-dir experiment/gas-budget-journal-cases
~/.venv/bin/python -S -m unittest discover \
  -s bin/zkgas/bench/tests -p test_gas_budget_journal.py -v
NATIVE_FIXTURE_BIN="$NATIVE_BIN" ZKGAS_QUOTE_BIN="$QUOTE_BIN" \
  ~/.venv/bin/python -S -m unittest discover \
  -s bin/zkgas/bench/tests -p test_gas_budget_journal.py -v
```

All users share a prestate runtime and empty calldata. Each original slot2 starts
at0, the parent checks0 and seeds5, the kernel checks5 then writes7, every bulk
round writes11 then7 and mixed additionally hashes the zero memory word between
those writes, and cleanup restores5 then0. A caught child REVERT returns32 bytes
containing the completed loop-derived marker; the parent requires status0,
returndata length32, the exact marker and final storage0. The once-only hash
guard and counter/tag/marker memory words remain disjoint. This prewarmed journal
panel does not cover child cold-first-access rollback.

The gas model separately declares intrinsic, static opcodes, memory expansion,
storage reads/writes, gross execution, original-zero restore refund19900 and the
`min(gross//5, 19900)` refund cap. User gas limits additionally preserve2301gas
above gross for the SSTORE sentry. Child layouts select
`ceil((full_gross + 2301)*64/63)` for EIP-150 headroom; controls use that same full
limit. These limits fit every transaction's remaining native31M-header gas.
Thirty million is the **charged benchmark budget**, not a claim that these gas
limits are admitted under a30M header. Native/quote execution remains explicitly
`evm-gas-only` under frozen modelc893, with no new node pricing limit.

Mandatory tests use bounded concrete executions plus decoded fixed-body/count
invariants for full inputs. Ordered reads, exact write transitions, actual hash
bytes, frame rollback, counter and marker checks catch read-key, write-key and
write-value mutations that can preserve successful receipts and opcode totals.
Optional frozen native probes check full occupancy, all receipts/counts, N0
refunds, caught-marker rejection, and the formerly insufficient gross+1024
SSTORE envelope as a retained diagnostic failure.

Keep all six ownN0 controls and every failure. Compare identical-proposal actual
SP1 PGU, complete proposal quote, and node per-block quotes/literal sum separately;
retain aggregate rounding diagnostics and both deficit flags. More than2B missing
PGU within the charged budget is an observed priority rule, not a projected result
or deadline. No prices, guest, SDK or prior producers change.

### Read-only gas-budget priority report

The maintained report command consumes an existing validated workload output root;
it runs no native execution, SP1 or network query:

```sh
~/.venv/bin/python -S bin/zkgas/bench/gas_budget_report.py \
  --output-root experiment/actual --output experiment/gas-budget-report.json
# Optional candidate node limit, in the new model profile's PGU units:
~/.venv/bin/python -S bin/zkgas/bench/gas_budget_report.py \
  --output-root experiment/actual --node-limit "$NODE_PGU_LIMIT" \
  --output experiment/gas-budget-with-node-limit.json
```

It verifies charged receipt/cumulative gas against native per-block/user totals,
uses canonical literal node sums from the validated quote, and retains full
proposal pricing and actual whole-proposal PGU. The default budget is30M and
high priority means strictly more than2B actual-minus-node PGU. It reports99%
occupancy separately; an underfilled block exceeding2B still counts. Sampled
maxima are not upper bounds. No eligible rows gives a null maximum.

Failed/incomplete measurements retain their failure and null actual/priority;
multi-block proposal observations remain whole and get no fabricated single-block
priority or block-limit admission result. Without `--node-limit`, admissibility
is unknown rather than silently applying the old Unzen limit. With it, the
single-block node-limit comparison remains separate from gas-only priority.
Output files are exclusive and original evidence is never rewritten. Independent
operand/input qualification remains required; a report cannot establish the
semantics of a wrongly constructed program.

The [first32-input gas-budget results](../../../docs/research/2026-11-02-gas-budget-composition.md)
retain all16 controls, literal node/full shortfalls and near-full measurements.
They leave heterogeneous transaction sequences and cold-access/rollback
combinations open; their measured maxima are not bounds for those combinations.

### Repeated child-access composition

`cold_frame_composition.py` creates12 unfitted inputs: cold-first child RETURN,
cold-first child REVERT and parent-prewarmed child REVERT, each at4/16 signed
user CALLs with an exact zero/full control pair. All users share empty calldata.

```sh
~/.venv/bin/python -S bin/zkgas/bench/cases/cold_frame_composition.py \
  --output-dir experiment/cold-frame-cases
~/.venv/bin/python -S -m unittest discover \
  -s bin/zkgas/bench/tests -p test_cold_frame_composition.py -v
NATIVE_FIXTURE_BIN="$NATIVE_BIN" ZKGAS_QUOTE_BIN="$QUOTE_BIN" \
  ~/.venv/bin/python -S -m unittest discover \
  -s bin/zkgas/bench/tests -p test_cold_frame_composition.py -v
```

The parent uses a fixed four-site self-DELEGATECALL body. Each child validates its
32-byte input tag, SLOADs absent slot2=0, and returns/reverts32 bytes from that read.
The parent checks status, returndata length and the actual output for every call,
its final loop counter and a final read of slot2. Only the prewarm profile reads
slot2 before the loop; no profile reads it between child calls. No SSTORE or refund
occurs. Separate memory locations hold the counter64, output128 and input tag256.

Native cold RETURN first warms the slot and retains warmth; child REVERT restores
its checkpoint, preserving an outer prewarm but undoing a child's newly introduced
warmth. New transactions reset EVM warmth even when the cached slot remains. The
concrete test oracle checks ordered keys/values, checkpointed warmth, cache/reset
separation, call/return data and per-transaction gas. Native receipts alone are not
a cold-access trace. In particular, coldness does not imply a DB reread or MPT
materialization; keep the actual quote's witness/path selectors separately.

WithN calls per user, whereN is divisible by4, exact charged gas is:

- cold RETURN: `23244 + 379*N + 55*(N//4)`;
- cold REVERT: `23244 + 2379*N + 55*(N//4)`;
- parent-prewarmed REVERT: `23369 + 379*N + 55*(N//4)`.

These include intrinsic21000, all guards/call/loop instructions, nine parent memory
words and one new child memory word per call. They are whole-user execution formulas,
not isolated child or SLOAD tariffs. The full selects the largest batch below29.99M
charged block gas including fixture Anchor167355; predictions are29,941,371..29,989,935.
Controls share the same full-selected gas envelope, gross+100000, sufficient for the
short child even under EIP-150. It fits every remaining native31M-header prefix.
Thirty million is the charged benchmark budget, not30M-header admission evidence.

Keep all six controls, semantic native-success mutations and native rejects; failed
or unmeasured samples have no invented PGU. Compare identical-proposal actual SP1PGU,
node per-block estimates/literal sum and the complete proposal estimate separately.
Frozen modelc893, SDK6.3, ELFb76 and explicit `evm-gas-only` remain unchanged. No new
node limit is selected. Strictly more than2B positive actual-minus-node PGU within
30M charged gas is high priority; finite measurements do not establish a bound,
production capacity or a proof deadline.

### Shared-account transaction-order composition

Generate24 shared-account transaction-order inputs:

```sh
~/.venv/bin/python -S bin/zkgas/bench/cases/order_composition.py --output-dir experiment/order-composition-cases
~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests -p test_order_composition.py -v
NATIVE_FIXTURE_BIN=/path/to/frozen/native ~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests -p test_order_composition.py -v
```

Each dirty/mixed journal combination packs4 or16 signed users, half paired-SWAP16
and half self-DELEGATECALL journal work ending in caught REVERT. Alternating,
density-first and journal-first order use one shared32-byte-word calldata table;
a genuine persistent phase slot4096 selects the next role and advances across
transactions. Every transaction observes its own storage originals: the initial
phase set costs20000 gas and subsequent clean nonzero updates cost2900. Journal
slot2 starts and ends zero per user. All phase, table, dispatch and state-root
work remains included in absolute same-proposal actual/node/full comparisons.

Each order keeps its own N0, with identical calldata/code layout and the same
full-selected transaction envelope. Full bounds are the largest integer batch
counts within each role's equal charged-gas allocation, and occupy29.7M..30M
charged whole-block gas including Anchor. Native header remains31M; all ordered
transaction envelopes fit the remaining header gas. This does not demonstrate
30M-header admission or select a node limit. Explicit gas records distinguish
intrinsic nonempty-calldata gas, the nonbinding candidate EIP-7623 floor,
gross execution, journal refund-cap effects, charged receipts and EIP-150/SSTORE
sentry envelope headroom.

The mandatory stdlib oracle checks bounded actual stack seeds, table/order,
per-transaction originals/cold reads, ordered writes, hashes, rollback frames
and derived markers. Full opcode totals use independently interpreted fixed
bodies and affine loop counts. Native-success mutations retain opcode counts
while violating order, phase key, stack seeds, a reverted bulk write key or a
final read key; their receipt success alone is insufficient semantic evidence.
The native report has no per-slot phase trajectory field: phase history relies
on the independent oracle and runtime guards, rather than state-root equality
alone. Keep all12 controls and native failures. SP1 measurements, when separately
qualified, must retain actual normalized PGU, each node block estimate and literal
sum, and the complete proposal estimate. Own-N0 differences remain diagnostic;
no per-user actual allocation, pricing refit, proof throughput or universal bound
is implied.

The [36-input results](../../../docs/research/2026-11-02-order-cold-frame.md)
retain both absolute deficits and the severe REVERT overquote tail. The latter
comes from the existing accepted5M/event reserve; covered observations are not
automatically accurate prices. All original controls and counterexamples remain.

The host fixture also accepts optional `program.prestate_contracts`, a nonempty
list of up to512 `{ "address": "0x…", "code_hex": "0x…" }` objects. Each address
has exactly20 bytes, is unique, exceeds the reserved low range0x0..0xffff and
avoids the primary runtime, sender, Golden Touch, Anchor, checkpoint and resolver
accounts. Ordinary deployed runtime must contain1..=24576 bytes and cannot start
with the reserved0xef prefix. Extra accounts start with nonce1, zero balance and
empty storage. This mode requires one block and explicit `transactions.kind =
"call"`, and excludes storage_state, access_list and authorizations. It raises
only this mode's request bound to32MiB; legacy requests retain4MiB.

A preparatory native pass records actual backing-database account, storage and
code reads, including reads in reverted frames. The fixture exports the canonical
multiproof for those targets with mutation support, deduplicates fetched code by
hash, and replays the minimized witness to check identical block, receipts,
poststate, opcode observations and read identities before ordinary proposal
validation. Unused code is absent. Trie sibling structure needed to authenticate
accessed targets can still commit to background accounts. Predeployment costs
are outside this measured block; preparatory native execution is not SP1 PGU.

Only this mode adds the optional `prestate_contracts` report object.
`configured_code_bytes` sums runtime bytes per configured address, while
`configured_unique_code_bytes` deduplicates hashes. `witnessed_account_count`
counts configured addresses actually loaded, and `witnessed_unique_code_count`
and `witnessed_code_bytes` count their code hashes actually fetched.
`loaded_account_target_count` includes system and absent-account reads;
`loaded_storage_target_count` counts actual raw storage targets. These are
execution targets, not encoded trie leaf counts. The total witness code counts
and bytes include system/primary code. Full/minimized witness node counts expose
the authenticated proof reduction. These diagnostics do not allocate actual PGU.

The multi-account code-loading producer uses this mode to vary 16/128/512 target
accounts, 32/4096/24576 code bytes and shared versus unique code hashes. Each of
the 18 settings has its own zero-call control and full-gas case, for 36 inputs.
The default uses CALL children. Four signed users cycle through targets; every
CALL child returns its own ADDRESS in
32 bytes and the parent checks status, return length, address and final counter.
The executed child prefix is identical; unreachable tails and fixed-width salts
vary loading/analysis work. Distinct accounts still execute separate calls and
have separate warmth; shared hashes only deduplicate code bytes and analysis.

```sh
~/.venv/bin/python -S bin/zkgas/bench/cases/multiaccount_code.py \
  --output-dir "$CASES_DIR"
~/.venv/bin/python -S bin/zkgas/bench/cases/multiaccount_code.py \
  --child-call-kind delegatecall --output-dir "$DELEGATE_CASES_DIR"
~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_multiaccount_code.py
NATIVE_FIXTURE_BIN="$NATIVE_BIN" ZKGAS_QUOTE_BIN="$QUOTE_BIN" \
  ~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_multiaccount_code.py
```

Use a host fixture built with this optional prestate interface. GuestInput remains
the ordinary vendor-neutral input; this producer requires no new guest format.
Use `workload.py screen` and `measure` with frozen artifacts as above. Preserve
literal node per-block quotes/sum, full proposal quote and actual same-proposal
SP1 PGU separately, including all controls and failed attempts. Full cases target
at least 99% of the 30M charged whole-block gas budget, including Anchor; headers
remain 31M. This selects no future node limit. The per-user gas envelope is the
same for each zero/full pair. A native-only one-account control and semantic
negatives supplement the matrix; they do not supply invented actual PGU.

`--child-call-kind delegatecall` emits a separate 36-input matrix with distinct
sample IDs and independently derived gas bounds. Top-level users remain ordinary
CALL transactions. The child still executes ADDRESS/MSTORE/RETURN32, but ADDRESS
returns the parent context under DELEGATECALL. The parent checks that context,
status, length and counter; independent target-order, code-hash and authenticated
account checks establish which account's code was loaded. Returned ADDRESS alone
does not establish code-target identity. These code-only children access no storage.

This comparison avoids the existing 1M PGU confirmed-CALL wrapper reserve masking
code-loading cost. Confirmed DELEGATECALL retains its existing 6000 PGU wrapper;
both also pay normal dispatch. Keep all CALL observations and their overquotes.
Freeze each matrix's source, canonical inputs and configuration separately before
SP1 sampling; running the second matrix does not reprice or rewrite the first.

The [72-observation research record](../../../docs/research/2026-11-02-multiaccount-code-loading.md)
keeps the two source/input freezes, CALL reserve masking, DELEGATECALL results,
all N0 gaps and small code-length growth discrepancies. Whole-block coverage
does not certify every marginal coefficient or the maximum possible code footprint.

The separate unreachable code-analysis catalog adds six tail profiles at4096 and
24576 bytes with512 unique targets and own-N0 pairs (24 requests). It composes the
same four-user DELEGATECALL parent and transaction envelopes. Only unreachable
target bytes and campaign metadata change. `stop`, `jumpdest`, `mixed` and
`push32` vary scanning/JUMPDEST work; `jumpdest-copy` ends with ADD and
`jumpdest-pad32` ends with PUSH32 to exercise the legacy copy/padding branches.
Every target starts with `3060005260206000f3`, then its tail, then an aligned
`PUSH4 <index+1> <terminal>` suffix. PUSH32 groups contain32 JUMPDEST immediate
bytes, which must not become jump-table entries. Under the retained
revm-bytecode43.0.0 analysis, the final STOP needs no padding, ADD needs one byte,
and the incomplete PUSH32 needs33 bytes including a synthetic STOP.

```sh
~/.venv/bin/python -S bin/zkgas/bench/cases/code_analysis_tail.py \
  --output-dir "$TAIL_CASES_DIR"
~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_code_analysis_tail.py
CODE_ANALYSIS_NATIVE_BIN="$NATIVE_BIN" CODE_ANALYSIS_NATIVE_LEVELS=zero,full \
  CODE_ANALYSIS_NATIVE_ARTIFACT_DIR="$TAIL_NATIVE_DIR" \
  ~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_code_analysis_tail.py
```

All output directories must be new. The catalog records the model ID, producer
and imported local producer hashes, profiles, lengths and levels. Generation
uses roughly350MiB for24 requests; leave additional room for canonical native
inputs and receipts. Native smoke is opt-in, uses the supplied frozen executable
without builds, and covers all six profiles at both lengths for the selected
levels (`zero`, `full`, or both). With a retention directory, keep native requests,
canonical JSON/bincode, reports and process results for inspection; without one,
each native case is removed before the next case to bound temporary disk usage.
The mandatory suite generates the catalog once and runs representative concrete
full/zero loops plus each profile's RETURN/context and independent tail checks.

Use this catalog with `workload.py screen` and `measure` and a fresh frozen
configuration/root as documented above, keeping native, quote, SDK6.3 estimator,
workload ELF and model unchanged. Qualify all24 canonical inputs and receipts
before freezing source and inputs for sequential guarded SP1 sampling. Retain
literal actual same-proposal SP1 PGU, each block node estimate and their literal
sum, and complete proposal estimate separately. Positive actual-minus-node PGU
strictly above2B within30M charged block gas is high priority; smaller differences
can remain TODOs. Content changes also change authenticated state, so contrasts
are whole-input observations. These probes establish no global bytecode bound,
price refit, generated proof or GPU throughput result. Preserve the prior72-case
campaign and publish any qualified new fixture under a separate immutable identity.

### Committed storage mutation

`storage_mutation.py` creates 24 inputs: update, delete, insert and mixed
insertion/deletion, each with 8192 or 16384 initialized storage entries and
zero, 64 or maximal-full mutations per user. Eight sequential signed ordinary
CALL users share a short runtime and empty calldata. Every mode/state pair has
its own N0 control, identical initialized background and full-selected gas
envelope. Output directories must be new.

```sh
~/.venv/bin/python -S bin/zkgas/bench/cases/storage_mutation.py \
  --output-dir "$STORAGE_CASES_DIR"
~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_mutation.py -v
STORAGE_MUTATION_NATIVE_BIN="$NATIVE_BIN" \
  ~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_mutation.py -k frozen_native -v
```

The optional smoke runs only insert/8192/N64 with the supplied frozen executable;
it performs no build or download. It checks nine successful receipts, exact
charged gas and the explicit-storage final-root/count diagnostic contract.
Mandatory tests use only the Python standard library, independently execute the
actual runtime for all 24 cases through all eight users, and check ordered raw
keys, values, cold/warm accesses, per-transaction originals, writes, concrete
final maps, loop counters, checksums, phase transitions, opcode counts and exact
gross/refund/charged gas. Successful equal-value wrong-range programs are
retained semantic negatives. A successful receipt alone does not qualify a case.
The smoke's root shape check is not independent root authentication; qualify
initial and final roots separately against the full raw-slot map before actuals.

Initially exactly raw slots 0 through S-1 contain 1. Slot 0 stores the next range
start, initially 1. User j processes keys `1+j*N+i` in ascending order and commits
`1+(j+1)*N` to slot 0. Update writes 2, delete writes 0, insert writes 2 to absent
counterparts at `1048576+1+j*N+i`, and mixed deletes each present key then inserts
its counterpart. Every operation guards the old value and reads the actual new
value into the checksum; final counter, checksum and phase checks precede STOP.
Phase reads/writes count toward gas, routes and proof targets even for N0.

Full N is the largest feasible integer under the 30M charged whole-block budget,
including the 167355-gas Anchor, the 8192 proof-target limit, valid present-key
ranges, the existing 15M per-user cap, and every remaining-header prefix. Each
candidate uses its own gross-user-gas plus 1000 envelope; the selected full
envelope is reused at all three levels. Header gas remains 31M. Tests also
execute a concrete N+1 bytecode candidate to establish that a real constraint
fails. The catalog separates gross gas, deletion refund counter, per-transaction
20% refund cap, charged gas and the envelope; SLOAD/SSTORE are priced once with
EIP-2200/2929/3529 rules. Full occupancy is a prediction until receipts establish
it; this does not show admission under a 30M header.

Use a host fixture supporting explicit storage-state CALL users and its optional
`storage_state.final_storage_root` / `final_storage_entry_count` report fields.
These report the actual common recipient's committed storage and are separate
from GuestInput. Generation uses roughly 9MiB for the complete source catalog.
Before actuals, qualify every canonical input, ordered signed transaction,
receipt, quote, opcode route and authenticated initial/final root, then freeze
source/host/input hashes alongside unchanged quote, SDK6.3, workload ELF and
model c893. Leave the existing 320MiB bounded-retention reserve plus room for
canonical inputs and results before guarded sequential SP1 execution.

Retain all eight controls and every failure. Compare actual same-proposal SP1
PGU, each node block estimate and its literal sum, and complete-proposal estimate
separately, preserving both deficit flags. More than 2B positive actual-minus-node
PGU within 30M charged block gas is high priority. These ordinary batch-storage
patterns are finite evidence; they imply no global MPT bound, fitted price,
per-user actual allocation, generated proof or GPU throughput result. Existing
producers, pricing, protocol limits, dependencies, guest and ELF remain unchanged.

### Exact-sized committed updates

`storage_mutation.make_sized_update_case(users=..., mutations_per_user=...,
raw_key_layout=None)` is a separate Python recipe helper. It reuses the existing
guarded update loop and assembler; old mutation/fanout `make_case` and `prepare`
requests and complete case rows retain their byte identities. Catalog source
and helper hashes still identify the actual producer source, so those provenance
fields change when the source changes.

```python
from storage_mutation import make_sized_update_case

row = make_sized_update_case(users=3, mutations_per_user=2)
# Optional compact layout; the caller must choose its protocol constants:
# row = make_sized_update_case(users=..., mutations_per_user=...,
#     raw_key_layout={'L1': chosen_l1, 'L3': chosen_l3})
request = row['request']
```

Counts are strict integers: 1..64 users and 0..64 updates each; booleans are
rejected. N0 is only an engineering control. Every user successfully updates
its own consecutive ordinal range from 1 to 2 on one shared prestate recipient,
checks its reads/counter/checksum and commits the next ordinal to raw phase slot0.
The helper uses one `prestate_contracts` entry with `initial_storage` and matching
`transactions.recipients`; it does not use the older eight-user `storage_state`
path. The unused primary runtime stays STOP and calldata remains empty.

Raw layout accepts exactly two strict U64 integers, `L1` and `L3`. It materializes
the raw SLOAD/SSTORE key as `k(i)=(i<<48)|(L1<<64)|(L3<<192)`, with L2=0 and
`i=1..users*N`. This is a compact algebraic layout, not an arbitrary 4096-key
vector or trie-hash input. All three body key sites use the same transform;
phase slot0 and its values remain ordinal. Every phase/body slot starts at1,
body slots end at2, with no refund or restore-to-original branch. Identity binds
the complete shape/envelope, ordered concrete raw keys and request. The runtime
and initial slots fit existing ProgramRequest fields; no guest/schema change is
introduced.

The raw transform adds36 gas per update to the sequential instruction stream.
The 64x64 pure-bytecode test predicts363551 charged gas/user,364551 envelope and
23434619 block gas including Anchor167355. These are engineering VM/source
checks, not native receipts or SP1 results. N0 reserves3000 beyond gross gas to
satisfy the same-value phase SSTORE's2300-gas sentry; positive N reserves1000.
Every ordered31M-header admission prefix and the30M charged block budget is checked.

Focused tests execute actual bytes for tiny1/2/3 shapes and the full64x64 state,
including raw key boundaries, original values, coldness, phase, checksum, exact
routes/gas/refunds and negative transforms/initial states. Run the ordinary
`test_storage_mutation.py` and `test_storage_fanout.py` suites with native opt-ins
unset. The helper does not prepare a formal catalog, select reference/successor
constants or invoke native/quote/SP1. Before any later sampling, independently
qualify signed user/Anchor order, canonical witness and initial/final roots.
Current model02223227 includes the128-PGU account-cache reserve; retain every
existing pricing term when later comparing literal node, full proposal and
actual SP1 workload. Neither per-map attribution nor underpricing is assumed.

### Initialized-storage fanout

`storage_fanout.py` creates the separate `unzen-storage-fanout-20261102` corpus:
24 inputs crossing 1/8/64 accounts, 8192/16384 aggregate initialized entries,
update/delete, and own-N0/full pairs. Each account starts with exactly raw slots
`0..S/A-1` equal to 1, including phase slot 0. All configured accounts share the
existing guarded `storage_mutation` runtime. The unused primary runtime stays
`0x00`. There are always 64 ordinary signed CALL users with empty calldata;
user `j` calls account `j%A`, starts at `1+floor(j/A)*N`, processes N consecutive
slots, and commits the next phase to that account. These users execute no
CALL/CALLCODE/DELEGATECALL/STATICCALL opcode; the Anchor remains separate.

```sh
~/.venv/bin/python -S bin/zkgas/bench/cases/storage_fanout.py \
  --output-dir "$FANOUT_CASES_DIR"
~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_fanout.py -v
STORAGE_FANOUT_NATIVE_BIN="$NATIVE_BIN" \
  ~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_fanout.py -k frozen_native -v
```

Output directories must be new; `--model-id` accepts a lowercase SHA256 and
defaults to frozen model c893. The catalog records request hashes, generator and
imported source hashes, explicit runtime/gas helper hashes, recipient and phase
schedules, aggregate/per-account counts, and gross/refund/charged gas separately.
`account_count`, `aggregate_state_size` and `state_size_per_account` describe the
account split. No observations are used for fitting.

Full N is derived from all constraints: `64*N+A<=8192` distinct account-slot
pairs, `(64/A)*N<=S/A-1` initialized body slots per account, the 30M charged
block budget including the 167355-gas Anchor, the 31M header at every prefix,
and each candidate's gross gas plus 1000 envelope within the 15M host and
EIP-7825 caps. The resulting N is 84 for update and 106 for delete, with charged
block gas 29986683/29918395 and envelopes 466927/582075. Zero/full pairs reuse
the full envelope. Every user writes N body slots and phase slot 0; aggregate
writes are `64*(N+1)`. Final initialized-entry count is S for update and
`S-64*N` for delete, and every account ends at phase `1+(64/A)*N`.

The mandatory standard-library suite executes all 24 actual bytecode programs
over separate rolling account maps, checking recipient order even for symmetric
N0 states, raw reads/writes, per-transaction originals/warmth/refunds, phases,
final maps/counts, routes and concretely executed N+1 infeasibility. It rejects
rotated/collapsed recipients, swapped storage owners, wrong phase/value and
malformed declarations. A changed update value with a matching changed checksum
can execute successfully and still fails intended semantics.

The opt-in native smoke runs two bounded full cases (8-account update and
64-account delete) against the supplied executable, with a 180-second timeout
per case and one temporary case retained at a time. It performs no builds or
downloads and checks all 65 receipts, charged gas, and declaration-ordered
`prestate_contracts.initialized_storage` initial/final root/count diagnostics.
Root shape/count checks do not independently authenticate roots or signed order.
Before actuals, independently review source and qualify all 24 canonical inputs,
real quotes, authenticated initial/final roots, all 64 signed destinations,
receipts and intended bytecode semantics. Preserve own-N0 controls and every
failure; freeze source/host/input hashes with unchanged quote, SDK6.3, workload
ELF and model. Compare actual same-proposal SP1 PGU, literal node block estimates
and sum, and complete-proposal estimate separately, retaining both deficit flags.
These finite inputs establish no universal storage-trie bound, price refit,
per-user actual allocation, generated proof or throughput result.

### Storage-root diversity

`storage_root_diversity.py` creates the separate
`unzen-storage-root-diversity-20261103` corpus: 32 inputs crossing
`background-values`/`disjoint-keys`, 8/64 accounts, 8192/16384 aggregate entries,
update/delete, and own-N0/full pairs. It reuses the accepted fanout runtime,
full-N solver, signed recipient declarations and gas accounting unchanged.
There are always 64 ordinary CALL users with empty calldata, the same
round-robin addresses, full N of 84/106, envelopes 466927/582075, a 31M header
and at most 30M charged gas including Anchor. The existing homogeneous corpus
is an immutable reference; this panel adds no one-account variant.

Let `A` be the account count, `K=S/A`, `V=64/A`, `F=fullN`, and `i` the
zero-based declaration-order account index. Each own-N0/full pair has identical
initialized maps:

- `background-values`: raw slots `0..K-1`, phase slot 0 equal to 1, slots
  `1..V*F` equal to 1, and untouched slots `V*F+1..K-1` equal to `i+2`.
  The salt boundary uses full N even for N0. Every accessed value and raw key
  stays the same as the homogeneous reference.
- `disjoint-keys`: `B_i=(i+1)*65536`, raw phase slot 0 equal to `B_i+1`, and
  raw slots `B_i+1..B_i+K-1` equal to 1; all other slots are absent.
  Actual body keys and phase control values shift; body values remain `1->2`
  for update or `1->0` for delete. User `j` starts at
  `B_(j%A)+1+floor(j/A)*N` and commits that start plus N to raw slot 0.

```sh
~/.venv/bin/python -S bin/zkgas/bench/cases/storage_root_diversity.py \
  --output-dir "$ROOT_DIVERSITY_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_root_diversity.py -v
STORAGE_ROOT_DIVERSITY_NATIVE_BIN="$NATIVE_BIN" \
  ~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_root_diversity.py -k frozen_native -v
```

Output creation is exclusive. The optional `--model-id` defaults to frozen
c893 and accepts a lowercase SHA256. The canonical catalog records generator,
imported source and runtime/gas helper hashes, and 32 distinct request hashes.
Row-level `homogeneous_reference_sample_id` identifies one of 16 existing
same-A/S/mode/N fanout references; `baseline_sample_id` names the new row's own
profile/A/S/mode N0. These references are identities, not new measurements.
Parameters include `profile`, `account_count`, `aggregate_state_size`,
`state_size_per_account`, and address-keyed `phase_base_by_account`,
`initial_phase_by_account` and `expected_final_phase_by_account` maps.
`start_phase_schedule`/`final_phase_schedule` give actual phases in transaction
order. There are no singular initial/final phase compatibility aliases.

The standard-library suite executes all 32 bytecode trajectories over isolated
rolling maps. Its literal formula oracle checks complete initial/final maps,
owner and recipient order, raw reads/writes, transaction originals and warmth,
refund caps, gas/envelopes, phases, counters/checksums and routes. It rejects
collapsed or wrong-profile maps, incorrect tail boundaries/salts, shifted phase
bases, wrong ranges/owners, rotated schedules and successful bytecode with a
changed body value and matching changed checksum. The optional native smoke
runs background-values/8-account/update/full and
disjoint-keys/64-account/delete/full with a 180-second timeout per case,
checking 65 receipts, charged gas, counts and A distinct diagnostic initial and
final storage roots. It builds nothing and is not independent qualification.

Before actual measurements, independently review source and qualify all 32
canonical signed recipient/receipt streams, authenticated whole-account and
per-account initial/final roots, canonical conservative mutation witnesses,
exact gas/envelope prefixes and same-input original real quotes. Require A
distinct initial and final storage roots per row; different roots alone do not
prove zero shared internal witness nodes. Freeze source/tools, all inputs and
quotes, and the 16 homogeneous reference identities before sequential bounded
execute-only measurements. Preserve controls, failures and both deficit flags;
compare whole-proposal actual SP1 PGU, literal node per-block ceilings/sum, and
complete proposal estimates separately. This finite validation does not fit
prices, allocate actual PGU to individual operations, or establish a global
trie bound, generated proof or throughput result.

### Heterogeneous storage restore and root rollback

`storage_root_journal.py` creates the separate 36-input
`unzen-storage-root-journal-20261103` corpus: `restore-stop`, `update-revert`,
`restore-revert` × 8/64 declared accounts × 8192/16384 aggregate initialized
entries × own-N0/small32/full. It reuses the disjoint-key request/map builder
and assembler/guards, and leaves the older producers unchanged. Each input
has one 31M-header block, 64 signed ordinary CALL users with empty calldata,
a STOP-only unused primary contract, and at most 30M charged block gas
including Anchor 167355. Each fixed 64-user full program uses maximal N;
this does not optimize arbitrary transaction compositions.

For account index `i`, `B_i=(i+1)*65536`, `K=S/A`, and `V=64/A`.
Raw phase slot0 starts at `B_i+1`; raw keys `B_i+1..B_i+K-1` contain1.
The complete initialized maps and signed gas envelopes match within each
own-N0/small/full triple. Every body iteration checks1, stores2 and reads2
into the checksum. Restore modes also store1 and read1 into the checksum.
The runtime checks counter N, checksum `3N` (restore) or `2N` (update), and
writes/read-checks phase+N before STOP or zero-range root REVERT.

- `restore-stop` advances ranges across visits. User `j` starts at
  `B_(j%A)+1+floor(j/A)*N`; committed final phase is `B_i+1+V*N`.
  All main values finish1. Distinct targets total `64*N+A`.
- Root REVERT repeats `B_i+1..B_i+N` on every visit, restoring the complete
  original map and each account's original storage root. Distinct targets
  total `A*(N+1)`. User receipts are legitimately included failures; Anchor
  succeeds. Whole-world state still changes through fees, nonces and Anchor.

| Mode | Full N | User gross | Refund before terminal | Effective refund | Charged/user | Shared envelope | Charged block |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| restore-stop | 101 | 578135 | 282800 | 282800 | 462508 | 580436 | 29767867 |
| update-revert | 84 | 465933 | 0 | 0 | 465933 | 468234 | 29987067 |
| restore-revert | 80 | 463397 | 224000 | 0 | 463397 | 465698 | 29824763 |

These full values apply to both A and S choices. The solver derives gas from
assembled executed counts, cold/warm reads and EIP-2200 writes. Dirty
`1->2->1` earns2800 per key before terminal. STOP applies the one-fifth cap;
root REVERT discards the refund. Each shared envelope reserves2301 above
full gross; the solver checks initialized ranges, at most8192 distinct
(account,rawslot) targets, the 15M/user cap, EIP-7825 and every admission
prefix. All full points exceed99% charged-budget occupancy; `near_full`
records occupancy independently from maximal N.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/storage_root_journal.py \
  --output-dir "$ROOT_JOURNAL_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_root_journal.py -v
STORAGE_ROOT_JOURNAL_NATIVE_BIN="$NATIVE_BIN" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_root_journal.py -k frozen_native -v
```

Use the existing managed Python environment as `PYTHON_BIN`. Output creation
is exclusive; `--model-id` defaults to frozen c893 and accepts only a lowercase
SHA256. The catalog records generator, imported source and helper hashes,
36 distinct canonical request hashes, and own-N0 identities. Address-keyed
`phase_base_by_account`, `initial_phase_by_account` and
`expected_final_phase_by_account` separate committed phases from ordered
`start_phase_schedule`/`attempted_final_phase_schedule`. Effective
`expected_user_refund_counter` and `expected_user_refund_counter_before_terminal`
are separate. Storage and route event counts include attempted work discarded
by REVERT; distinct-body-key/visit/repeated-visit counts expose range reuse.

The stdlib oracle executes all36 actual byte streams over64 isolated rolling
maps each. It checks complete maps, exact owner/range/value/original/warmth
traces, memory, terminal, gas, refunds, routes and independently executed N+1
maximality. The shared bounded test VM adds only zero-offset/zero-length root
REVERT, retaining attempted traces and preterminal refunds while restoring the
caller's state dictionary in place; nonzero ranges are explicitly unsupported.
Existing STOP results are unchanged. Negatives include matching-terminal but
wrong values/checksums, recipients, phases, owner, warmth, refund and maps.
The optional native smoke checks the frozen binary SHA256
`ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec`,
then runs restore-stop/A8/full and restore-revert/A64/full plus corresponding
wrong-value/checksum mutants, with180 seconds per specimen. It checks65
receipts, charged gas and A distinct diagnostic initial/final roots, including
per-account equality after rollback. Mutants pass native execution with the
expected receipts but fail the literal semantic oracle. No rebuild is used.

Independent source review and all36 native/signature/authenticated-root,
canonical conservative mutation-witness and original real-quote gates precede
source/tool/input/quote freeze and actuals. Keep the same frozen pricing,
SDK6.3, workload ELF and host/guest; preserve all controls and failures.
Compare actual same-proposal SP1 PGU, literal node block estimates/sum and
complete-proposal estimate separately. This finite panel establishes no global
coverage percentage, trie bound, price refit, per-operation actual allocation,
generated proof or throughput claim.

### Few long root-REVERT calls

`storage_long_revert.py` creates the separate 16-input
`unzen-storage-long-revert-20261103` corpus: update/restore root REVERT ×
2/4 users and accounts × 8192/16384 aggregate initialized entries × own-N0/full.
Each signed top-level CALL visits its own predeployed contract once, with empty
calldata and no user CALL wrapper. It reuses the unchanged journal runtime/gas
helpers and builds a fresh request from the top-level CALL schema. For account
`i`, `B_i=(i+1)*65536`, `K=S/U`, phase slot0 is `B_i+1`, and raw slots
`B_i+1..B_i+K-1` contain1. Complete maps and signed envelopes match each own
N0/full pair. The guarded body attempts `1->2` or `1->2->1`, verifies counter,
checksum and phase+N, then executes zero-range root REVERT. Complete contract
maps and storage roots restore; whole-world fees, nonces and Anchor change.
All user receipts fail legitimately; Anchor succeeds.

| Mode | Users | Full N | Gross = charged/user | Preterminal refund | Shared envelope | Charged block | Attempted pairs |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| update-revert | 2 | 2844 | 14911773 | 0 | 14914074 | 29990901 | 5690 |
| update-revert | 4 | 1419 | 7453323 | 0 | 7455624 | 29980647 | 5680 |
| restore-revert | 2 | 2725 | 14915677 | 7630000 | 14917978 | 29998709 | 5452 |
| restore-revert | 4 | 1360 | 7457317 | 3808000 | 7459618 | 29996623 | 5444 |

These full values apply to both S choices. N0 gross/charged per user is23474;
its charged block is214303 with2 users or261251 with4. Effective and applied
refunds are zero, including the restore mode's discarded `2800*N` counter.
The solver checks <=30M charged block gas including Anchor167355, the31M
header, each candidate's own gross+2301 envelope,15M/user and EIP-7825 caps,
every ordered admission prefix, initialized ranges, and at most8192 distinct
attempted `(account,rawslot)` targets. Full/N0 share the selected full envelope;
actual charged occupancy is recorded independently. Reads, writes and opcode
counts retain attempted work after rollback.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/storage_long_revert.py \
  --output-dir "$LONG_REVERT_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_long_revert.py -v
STORAGE_LONG_REVERT_NATIVE_BIN="$NATIVE_BIN" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_long_revert.py -k frozen_native -v
```

Use the existing managed Python environment as `PYTHON_BIN`. Output creation
is exclusive and the model ID must be a lowercase SHA256, defaulting to c893.
The catalog records generator, transitive imported-source and helper identities,
canonical request hashes, exact phase schedules and expected failed receipts.
The stdlib oracle executes all16 actual byte programs, checks complete maps,
raw keys/owners/originals/warmth, gas, refunds, routes and memory, and executes
each full N+1 under a sufficient20M envelope before evaluating its own candidate
constraints. It checks the SSTORE gasleft>2300 sentry. Matching-terminal wrong
owner/domain/value/checksum/refund/phase/map mutants fail the semantic oracle.
The opt-in native test checks frozen SHA256
`ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec`
and runs one full U2/S8192 specimen per mode plus matching-value/checksum
mutants, with180 seconds per specimen. Native-accepted mutants still fail the
literal semantic oracle; native acceptance alone does not qualify the panel.

All16 independent native/signature/authenticated-root/canonical-witness and
original real-quote checks precede freeze and sequential actuals. Preserve
controls and failures, frozen pricing, SDK6.3, ELF and host/guest inputs. Compare
actual same-proposal SP1 PGU, literal node block estimates/sum and complete
proposal estimates separately. This finite composition panel changes packing
and existing REVERT allowances; it makes no isolated opcode-price, refit,
per-operation actual allocation, universal trie bound, proof or throughput claim.

### Many-key warm dirty writes

`storage_dirty_keys.py` creates the separate16-input
`unzen-storage-dirty-keys-20261103` corpus: STOP/root REVERT ×
1/16/256/1024 keys per account × own-R0/maximal uniform full R. Four signed
users each CALL one distinct predeployed contract once with empty calldata and
one shared runtime. Account i has `B_i=(i+1)*65536`, read-only raw phase0
`B_i+1`, and K raw keys `B_i+1..B_i+K` initialized to1; all other slots are
absent. Each own-R0/full pair shares complete maps and the full signed envelope.

The guarded runtime seeds **all** K keys `1->5`, then visits each key for R
warm dirty `5->7->5` pairs in64-pair batches plus the exact tail, and finally
cleans **all** keys `5->1`. No per-pair SLOAD or nested CALL is added. Actual
per-key round counters accumulate into marker `K*R`; phase0 is read-checked
and never written. Memory stays within192 bytes. Both terminals restore every
complete contract map/root; whole-world fees, nonces and Anchor change.
STOP succeeds and keeps the `2800*K` refund counter subject to the20% cap;
root REVERT fails legitimately and discards that counter. R0 still seeds and
cleans every key. Attempted reads/writes/opcodes remain after REVERT.

| Terminal | K | Full R | Gross/user | Applied refund | Charged/user | Shared envelope | Charged block | Near full |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| STOP | 1 | 33955 | 7460941 | 2800 | 7458141 | 7463242 | 29999919 | yes |
| STOP | 16 | 2108 | 7502220 | 44800 | 7457420 | 7504521 | 29997035 | yes |
| STOP | 256 | 117 | 8133052 | 716800 | 7416252 | 8135353 | 29832363 | yes |
| STOP | 1024 | 12 | 8966076 | 1793215 | 7172861 | 8968377 | 28858799 | no |
| REVERT | 1 | 33942 | 7458113 | 0 | 7458113 | 7460414 | 29999807 | yes |
| REVERT | 16 | 2095 | 7456882 | 0 | 7456882 | 7459183 | 29994883 | yes |
| REVERT | 256 | 104 | 7407554 | 0 | 7407554 | 7409855 | 29797571 | yes |
| REVERT | 1024 | 5 | 7403458 | 0 | 7403458 | 7405759 | 29781187 | yes |

Storage gas per user is `2200+K*(5600+200*R)`, plus21000 intrinsic,
18 memory gas, and ordinary opcode gas from executed route counts. Literal
access pairs are `4*(K+1)` and literal write pairs are `4*K`; phase0 only
belongs to the first set. Full resource counters can use conservative witness
semantics, so qualify their actual definitions before comparing them with
literal write sets. The solver keeps the31M header, includes Anchor167355 in
the<=30M charged budget, limits each gross+2301 envelope to15M and the
EIP-7825 cap, and checks every ordered header prefix and<=8192 access targets.
The reserve establishes `gasleft>2300` at every SSTORE. Maximal uniform R is
not arbitrary block maximality: STOP/K1024 R+1 would charge29573143 including
Anchor but its last admission prefix lacks413305 gas for its own gross+2301
envelope. Uniform-R granularity also limits occupancy. `near_full` truthfully
means charged block>=29700000; no cap or header is waived.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/storage_dirty_keys.py \
  --output-dir "$DIRTY_KEYS_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_dirty_keys.py -v
STORAGE_DIRTY_KEYS_NATIVE_BIN="$NATIVE_BIN" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_dirty_keys.py -k frozen_native -v
```

Use the existing managed Python environment as `PYTHON_BIN`. Output is
exclusive; the model ID must be a lowercase SHA256, defaulting to c893. The
catalog records generator, transitive source, helper and canonical request
identities, exact phase/key schedules, counters, refunds, routes and receipts.
The standard-library oracle executes all16 actual byte trajectories, complete
maps, raw originals/warmth and exact gas. It tests0/1/63/64/65 rounds across
batch and tail boundaries, sentry and all prefixes, and executes each R+1
under a sufficient20M envelope
before evaluating that candidate's own bounds. Semantic negatives include
wrong dirty values, key/owner/domain, skipped batch/tail, marker, phase writes,
refund/terminal and stale access/write metadata. Large event streams are
compared by length and per-record mismatch.

The native opt-in verifies the unchanged binary SHA256
`ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec`
and runs full STOP/K1 and full REVERT/K1024 plus their dirty7->9 mutants,
with180 seconds per specimen. It checks5 receipts per specimen, exact charged
gas and4 distinct original/final diagnostic roots with per-account equality.
The mutants remain native-accepted but fail the literal semantic oracle.
All16 independent native/signature/receipt/whole-world/authenticated-root,
canonical mutation-witness, original-quote and counter gates precede freeze
and sequential SP1 actuals. Preserve controls/failures and frozen model,
SDK6.3, ELF, host/guest and pins. Compare literal same-proposal actual PGU and
node ceilings/sum, with complete-proposal estimate separately. This finite
panel makes no price fit, isolated-operation allocation, global/production
bound, generated-proof or GPU claim.

### High-limb storage-key distributions

`storage_key_shift.py` creates24 **new** inputs in
`unzen-storage-key-shift-20261103`: STOP/REVERT × K16/1024 × index shifts64/128/192
× own R0/full. It reuses the authoritative `storage_dirty_keys` runtime, request
construction, gas accounting and full-R solver. The new catalog separately lists
**eight existing** shift0 references by original sample ID, canonical request
SHA256 and runtime SHA256. Join their original observations and source/config/
ELF/actual/quote identities; do not rerun or count references as newly sampled.

Four distinct users visit four accounts once. Account i has read-only raw slot0
holding `start=(i+1)*65536+1`; its K body slots are `start+(j<<shift)`, j0..K-1,
initialized1. The keys are distinct and below2^256. Request storage keys and
catalog slot schedules use hexadecimal strings; the catalog also records the
formula. There is no custom hashed-slot injection or collision claim. Each seed,
dirty and cleanup traversal shifts its index once with fixed-width PUSH3/SHL
before adding start. The already computed key pointer is reused for all dirty
writes. Seed `1->5`, R dirty `5->7->5` pairs, cleanup `5->1`, original values,
warmth, counters/marker,64-pair batching/tail and terminal semantics stay the same.
Default shift0 emits no new instructions and all16 preceding request/runtime/core
metadata identities are protected by immutable pre-extension hash goldens.

SHL costs3 gas plus3 for its PUSH, executed3K times/user: shifted ordinary gas
increases by18K at fixed K/R. All three shifts have the same executed opcode/gas
shape at fixed terminal/K/R, but different raw keys, code/trie hashes and witness
bytes. Comparisons remain whole-input measurements. Full R is solved again
under every unchanged limit and all ordered prefixes:

| Terminal | K | Full R | Gross/user | Applied refund | Charged/user | Shared envelope | Charged block | Near full |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| STOP | 16 | 2108 | 7502508 | 44800 | 7457708 | 7504809 | 29998187 | yes |
| STOP | 1024 | 12 | 8984508 | 1796901 | 7187607 | 8986809 | 28917783 | no |
| REVERT | 16 | 2095 | 7457170 | 0 | 7457170 | 7459471 | 29996035 | yes |
| REVERT | 1024 | 5 | 7421890 | 0 | 7421890 | 7424191 | 29854915 | yes |

Values apply to each shift64/128/192. STOP/K1024 remains below99% charged
occupancy because R+1 violates its own final admission prefix; no cap is waived.
Own R0/full initial maps and signed envelopes agree. Each terminal restores the
complete per-account maps/roots; whole-world fees, nonces and Anchor still change.
Read access pairs are `4*(K+1)` and write pairs are `4*K`; raw slot0 is read-only.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/storage_key_shift.py \
  --output-dir "$KEY_SHIFT_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_key_shift.py -v
STORAGE_KEY_SHIFT_NATIVE_BIN="$NATIVE_BIN" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_key_shift.py -k frozen_native -v
```

Use the existing managed Python environment as `PYTHON_BIN`. Output is exclusive,
and the model ID must be a lowercase SHA256. The stdlib oracle independently
executes all24 concrete byte trajectories, checks high raw-slot maps and ordered
per-record read/write events, exact gas/refunds/routes and counters/marker, and
executes every R+1 under a sufficient20M gas envelope before testing that
candidate's own bounds. It covers SHL operand order,0/1/63/64/128/192/255/256+
shifts and U256 truncation, strict APIs/provenance, the16 old default goldens and
eight reference identities, and wrong limb/key/domain/value/marker/phase-write
negatives. The bounded interpreter gains only ordinary SHL support.

The opt-in verifies unchanged native SHA256
`ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec`
and runs full STOP/K16/shift64 and full REVERT/K1024/shift192 plus their
native-accepted dirty7->9 semantic negatives, at180 seconds/specimen. All24
independent native/signature/receipt/whole-world/authenticated per-account-root,
canonical mutation-witness, original quote and counter gates precede source/input/
tool/resource freeze and sequential SP1 actuals. Report24 new literal observations
and64 diagnostics:12 own-R0 increments,12 terminal contrasts,16 adjacent-shift
contrasts and24 new-versus-exact-unshifted-reference contrasts. Preserve explicit
same-R/K/charged-gas/signed-envelope/authenticated-storage-support markers; storage
support equality uses canonical per-address selected node hash sets/digests,
with underlying identities retained. Global/account witness differences remain
separate. Preserve controls, failed attempts and frozen prices/model/SDK/ELF/host/
guest/pins; no isolated-operation actual allocation, pricing fit, global/production
bound, generated proof or GPU claim follows from this finite panel.

### Caught nested storage calls

`storage_caught_nesting.py` creates16 **new** inputs in
`unzen-storage-caught-nesting-20261103`: leaf STOP/REVERT × K16/1024 × CALL
wrapper depth1/4 × own R0/full. Four signed users each visit one disjoint chain
once. Depth counts wrappers; total accounts are `4*(depth+1)`, with exactly four
storage owners. The leaf addresses, raw phase0, initialized body maps and emitted
leaf bytes come from unchanged `storage_dirty_keys` helpers. Each wrapper owns
an explicit empty map, encodes its next target using PUSH20, and uses zero-value,
empty-input/output CALL with a12M gas request. Child gas is actually capped by
EIP150 at every frame. The innermost wrapper checks leaf status1 for STOP or0
for REVERT; all other wrappers expect1. Every wrapper also checks empty
RETURNDATASIZE and then STOPs, so all four top-level user receipts succeed.
Completed/attempted counters retain leaf work even on caught REVERT. Terminal
counts include every wrapper STOP plus four leaf STOPs or four leaf REVERTs.

A wrapper executes21 ordinary gas before CALL and46 after it; its cold target
costs2600, for2667 gross gas. These values are derived from the assembled bytes
and independently checked by the frame oracle. Let `C=leaf_gross-21000`.
Transaction gross is `21000+C+2667*depth`. Successful leaf refunds propagate to
the transaction; leaf REVERT discards its refund. The refund cap is applied once
at transaction gross/5, without child intrinsic gas or a child-level refund cap.
The signed envelope is solved from `T0=C+2301` and
`Tj=2621+max(ceil(64*Tprev/63),Tprev+46)`, then `21000+Tdepth`. The own R0 input
retains exactly the full input's envelope and initial maps.

| Leaf | K | Depth | Full R | Gross/user | Applied refund | Charged/user | Envelope | Charged block |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| STOP | 16 | 1 | 2107 | 7501399 | 44800 | 7456599 | 7622385 | 29993751 |
| STOP | 16 | 4 | 2105 | 7502424 | 44800 | 7457624 | 7990692 | 29997851 |
| STOP | 1024 | 1 | 12 | 8968743 | 1793748 | 7174995 | 9113020 | 28867335 |
| STOP | 1024 | 4 | 11 | 8753512 | 1750702 | 7002810 | 9323126 | 28178595 |
| REVERT | 16 | 1 | 2094 | 7456061 | 0 | 7456061 | 7576328 | 29991599 |
| REVERT | 16 | 4 | 2092 | 7457086 | 0 | 7457086 | 7942407 | 29995699 |
| REVERT | 1024 | 1 | 5 | 7406125 | 0 | 7406125 | 7525599 | 29791855 |
| REVERT | 1024 | 4 | 5 | 7414126 | 0 | 7414126 | 7896654 | 29823859 |

Full R is the maximal uniform integer under the unchanged30M charged block
budget including167355 Anchor gas,31M header and every ordered admission prefix,
15M user envelope/EIP7825, SSTORE sentry,8192 selected storage targets and24576
code bytes. Both STOP/K1024 rows stay below99% occupancy because R+1 violates
the final transaction admission prefix. Their limits are not waived; this is
not a global packing-capacity result. Every leaf and wrapper map/root restores;
whole-world user fees/nonces and Anchor still change. Empty wrapper roots
require authenticated world/account/code proofs but introduce no storage targets.
Catalog roles distinguish these accounts and retain exact per-account maps,
wrapper targets, code SHA256s, routes, all-frame terminals and gas/refund details.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/storage_caught_nesting.py \
  --output-dir "$CAUGHT_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_caught_nesting.py -v
STORAGE_CAUGHT_NATIVE_BIN="$NATIVE_BIN" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_caught_nesting.py -k frozen_native -v
```

Use the existing managed Python environment as `PYTHON_BIN`. The output is
exclusive and model ID must be a lowercase SHA256. The test-only bounded frame
interpreter executes actual wrapper bytes, using the unchanged public
`test_storage_mutation.execute` oracle for leaf frames with the child intrinsic
offset removed. It checks all16 complete ordered leaf streams, maps, status and
returndata guards, frame forwarding/returned gas, refunds and literal routes;
K16 exercises R0/1/63/64/65, and all eight shapes execute full/R+1 under a sufficient
envelope before checking candidate bounds. K1024/R63 exceeds the fixed12M child
request and is outside that executable boundary subset. Wrong target, expected
status, requested gas, CALL value/ranges, leaf values and marker specimens must
fail intended semantics. The opt-in verifies native SHA256
`ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec`
and runs full STOP/K16/depth1 and caught REVERT/K1024/depth4, plus two dirty7->9
specimens accepted by native execution but rejected by the literal trajectory,
at180 seconds/specimen. This focused tooling check does not replace the all16
independent signature/receipt/whole-world/authenticated-root/canonical-witness/
original-quote/attempted-resource-counter gates required before measurements.

The catalog freezes generator, source/helper identities and eight exact old
direct references by original request/runtime SHA256. Join their existing
source/config/input/quote/actual identities; do not rerun or count them as new
measurements. The32 diagnostics comprise eight own-R0 increments, eight leaf
terminal contrasts, eight depth contrasts and eight depth1-versus-old-direct
contrasts. Preserve exact scaled quote differences and explicit same-R/K/gas/
signed-envelope/per-owner authenticated selected-storage-support flags. Storage
support compares the four leaf owners only, using canonical selected node hash
sets/digests. Parent addresses, code hashes and global/account/code support
differ from old direct references, preventing isolated frame-cost attribution.

Keep c893 model/native/quote/SDK6.3/ELFb76, host/guest and dependency pins fixed;
freeze all16 inputs/quotes/source/tool/resource policies before sequential actuals.
Apply6GiB sampled PSS/600s SP1,180s native,1800s quote (max two),16MiB log and192MiB
output bounds and the576MiB free-space gate. Retain every failed setup/measurement
and explicit retry. Compare same-proposal SP1 PGU (`SDK gas*10//191`) against the
literal sum of block node-known PGU ceilings, reporting the complete proposal
estimate separately. Positive actual-minus-node above2B under30M charged gas is
high priority; smaller deficits remain explicit low-priority TODOs. This finite
panel makes no price fit, per-operation actual allocation, global/IID/production
bound, generated-proof or throughput claim. Raw recovery stays private until
separately packaged as immutable Atlas fixtures.

### Shared-code delegated storage calls

`storage_delegated_nesting.py` adds16 inputs in
`unzen-storage-delegated-nesting-20261103`: leaf STOP/REVERT × K16/1024 ×
DELEGATECALL depth1/4 × own R0/full. Four successful signed top users each enter
one disjoint wrapper chain. The four chains share **one** leaf code account;
configured account count is `4*depth+1`. Exactly the four top wrappers own
storage. Every top wrapper starts with the same raw map: phase slot0 contains
65537, body keys65537 through65536+K contain1. Intermediate wrappers and the
shared leaf code account have explicit empty storage. The unchanged dirty leaf
bytes perform seed1→5, R dirty5→7→5 pairs and cleanup5→1 in each top wrapper's
storage context. All maps restore, including caught leaf REVERT. Equal storage
roots across owners are expected and do not identify the account domain of a
storage proof.

The common `storage_caught_nesting` helpers accept keyword-only
`call_kind='call'|'delegatecall'`. The default preserves all16 prior CALL
construction requests, runtime identities and full core metadata; tests bind
immutable identities captured before these helper changes. DELEGATECALL emits
six arguments, with no value argument, a PUSH20 target and PUSH3 for other
operands. Empty input/output, a12M request, child-status and empty-returndata
guards remain explicit. Every target is cold within its transaction; shared
leaf code across different transactions does not share warmth. Each delegated
wrapper costs18 ordinary gas before the call,2600 for its target and46 after,
for2664 gross gas. Child intrinsic gas is removed, successful leaf refunds
propagate, caught leaf REVERT discards its refund, and the cap applies once to
the whole user transaction. The effective user counter before terminal is0 on
caught REVERT; the separately recorded leaf counter is2800K.

For leaf execution `C=leaf_gross-21000`, user gross is
`21000+C+2664*depth`. The signed envelope follows `T0=C+2301`,
`Tj=2618+max(ceil(64*Tprev/63),Tprev+46)`, then `21000+Tdepth`.
Full R is solved afresh under the same31M header,30M charged block including
167355 Anchor gas, all admission prefixes,15M user/EIP7825,12M child request,
SSTORE sentry,8192 selected storage targets and24576 code-byte limits. R0 keeps
its own full case's envelope and initial maps. Both STOP/K1024 full cases remain
below99% charged occupancy because the next uniform R fails admission; this
finite selector does not measure arbitrary packing capacity.

| Leaf | K | Depth | Full R | Gross/user | Applied refund | Charged/user | Envelope | Charged block |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| STOP | 16 | 1 | 2107 | 7501396 | 44800 | 7456596 | 7622382 | 29993739 |
| STOP | 16 | 4 | 2105 | 7502412 | 44800 | 7457612 | 7990680 | 29997803 |
| STOP | 1024 | 1 | 12 | 8968740 | 1793748 | 7174992 | 9113017 | 28867323 |
| STOP | 1024 | 4 | 11 | 8753500 | 1750700 | 7002800 | 9323114 | 28178555 |
| REVERT | 16 | 1 | 2094 | 7456058 | 0 | 7456058 | 7576325 | 29991587 |
| REVERT | 16 | 4 | 2092 | 7457074 | 0 | 7457074 | 7942394 | 29995651 |
| REVERT | 1024 | 1 | 5 | 7406122 | 0 | 7406122 | 7525596 | 29791843 |
| REVERT | 1024 | 4 | 5 | 7414114 | 0 | 7414114 | 7896640 | 29823811 |

The bounded actual-byte interpreter records `code_address` and
`storage_address` separately. The four owners require `4*(K+1)` distinct
account/slot access pairs and `4*K` write pairs, even though grouping by the
shared leaf code address would incorrectly report only `K+1` slots and K writes.
Catalog roles distinguish top wrappers with storage, intermediate empty
wrappers and the shared leaf code owner with no storage. Phase/body maps,
schedules and selected storage targets all name actual top storage owners.
The catalog includes16 immutable prior CALL references matched by leaf mode,
K, depth and workload level; those identities are not new measurements.
Actual account domains, raw key/value maps, code/account/global support and
signed envelopes can differ from CALL, so that comparison is whole-input
validation and does not isolate the cost of an opcode substitution.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/storage_delegated_nesting.py \
  --output-dir "$DELEGATED_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_delegated_nesting.py -v
```

The optional `STORAGE_DELEGATED_NATIVE_BIN` gate uses an already frozen native
binary, with retained artifacts under a fresh
`STORAGE_DELEGATED_NATIVE_OUTPUT` directory. It runs two full positives and two
native-accepted dirty-value mutants that the intent oracle rejects. All16
independent native/signature/receipt/world/account-specific storage-root/
canonical-witness/original-quote/context-counter checks must precede freeze and
SP1 actuals. This producer changes no prices, model, Rust, guest, SDK, ELF or
pins, and makes no global, production, proof or throughput claim.

`cases/storage_warm_rollback.py` adds32 finite caller-warmth and rollback
controls: leaf or ancestor exit, K16 or256, delegated depth2 or4, caller
prewarming off or on, and own R0/full. Four successful users visit disjoint
wrapper chains sharing one leaf CODE account. Each top wrapper owns an
identical phase0=65537/body65537..65536+K=1 map; intermediate wrappers and
leaf CODE have empty maps. DELEGATECALL keeps the four storage owners distinct.
Logical attempted accesses require `4*(K+1)` account/slot pairs and writes
require `4*K`; the read-only phase is excluded from write support.

TOP optionally reads and guards every phase/body value before calling, then
always reads and guards them after rollback. Both traversals use a guarded
ascending loop and one32-byte memory word, separately from the leaf's192 bytes.
Caller prewarming survives the lower checkpoint: initial leaf reads and post
reads cost100 per slot when prewarmed. Without prewarming both are cold2100
reads because the reverting subtree's new warmth is discarded. The original
leaf kernel is unchanged. Leaf exit catches its REVERT in the innermost
wrapper. Ancestor exit lets the leaf STOP, then reverts the innermost wrapper;
its parent catches0. Every user has exactly one REVERT, depth STOPs and depth
DELEGATECALLs. Leaf preterminal refund is2800K; a successful leaf passes that
counter only as far as the reverting ancestor. Every top preterminal counter,
effective refund and applied transaction refund is0. Storage maps restore;
Anchor effects, transaction fees and sender nonces still change world state.

Ordinary top traversal costs are derived from emitted assembly counts; storage
fees and the one-time3-gas memory expansion are separate. Each ordinary
wrapper has18 pre-call gas, a2600 cold target and46 post-call guards. Moving
the6-gas empty REVERT operands from leaf to ancestor preserves whole-user
gross gas and aggregate opcode counts at fixed K/depth/warmth/R. For actual
leaf execution `C`, use `Tleaf=C+2301` and
`Ti=pre_i+max(ceil(64*Tchild/63),Tchild+post_i)`, including top traversal
fees/memory in their actual regions. The selected signed envelope is the
maximum of the two exit modes at their common maximal full R; own R0 retains
that same envelope and initial map. Selection enforces the31M header,30M
charged block including167355 Anchor gas, every ordered admission prefix,
15M user/EIP7825 caps,12M child requests, SSTORE sentry,8192 storage targets,
24576 code bytes and R0..65536. K256 replaces K1024 because four obligatory
cold-postcheck K1024 R0 controls would exceed the charged budget. Reported
occupancy describes this uniform selector, not arbitrary packing capacity.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/storage_warm_rollback.py \
  --output-dir "$WARM_ROLLBACK_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_storage_warm_rollback.py -v
```

The separate bounded frame oracle interprets actual loops/SLOAD/DELEGATECALL/
REVERT bytes, snapshots storage/refund/account warmth/slot warmth per frame,
and retains attempted work across rollback. Its explicit20000-step wrapper
bound supports both top traversals at K256; the existing CALL/F4 oracle's64
step bound and the leaf oracle's default result shape remain unchanged.
Regression goldens preserve the32 prior reviewed CALL/F4 request and complete
core metadata identities. Optional `STORAGE_WARM_NATIVE_BIN` with a fresh
`STORAGE_WARM_NATIVE_OUTPUT` archive runs two full positives and two
native-accepted dirty9 mutants that literal intent rejects. Independent all32
native, signature, receipt, whole-world, authenticated owner-root, canonical
witness, original quote and attempted-counter gates precede freeze and SP1
actuals. These are whole-input comparisons; warmth/exit changes do not isolate
per-opcode actual allocations. Prices, model, Rust, guest, SDK, ELF and pins
are unchanged; no global, production, proof or throughput claim follows.

`cases/transient_storage.py` adds eight transient-storage inputs: root STOP or
REVERT, K16 or4096 body keys, and each input's own R0 or generated-full workload.
Four users call the same code account, whose persistent initial storage is empty.
Each transaction guards transient sentinel0=0, stores sentinel1, seeds keys1..K
from0 to5 with pre/post reads, performs R dirty5->7->5 pairs per key with pre/post5
reads, cleans every body key5->0 with pre/post reads, then checks sentinel1 and
loop/round/marker counters. STOP retains the sentinel until transaction finalization;
the next user's entry guard verifies reset. Root REVERT restores transient changes
while attempted instructions, values and journal entries remain observable.

All four users share one transient owner: the union has K+1 owner/slot pairs,
with four independent transaction attempt sets and reset boundaries. This is not
four separate owners or persistent storage witness support. Generic
`aggregate_state_size` and `state_size_per_account` both remain0 persistent
entries; `peak_transient_state_entries_per_transaction` explicitly records K+1.
The request declares
one prestate code account, empty persistent storage and four explicit recipients
naming that account. Transient operations must not inject phantom persistent MPT
targets. Authenticate code/account and whole-world evidence, retain the empty
persistent roots, and derive any observed persistent tracer selectors from their
source/system scope before interpreting them. Fees, sender nonces and Anchor
execution still change whole-world state.

Per user, the guarded program executes2+6K TLOADs and1+K*(2+2R) TSTOREs, uses192
memory bytes and performs no SLOAD/SSTORE. Each transient opcode costs100 EVM gas,
without cold surcharges, storage refunds or the2300-gas SSTORE sentry. Ordinary
instruction gas comes from executed counts; memory expansion18 and intrinsic21000
are separate. The1000-gas envelope reserve is fixture policy. Each own R0 control
retains its full candidate's selected envelope. Full means maximal uniform R under
the30M charged block cap including167355 Anchor gas,31M header, every ordered
admission prefix,15M user fixture cap,16,777,216 EIP-7825 cap and24576-byte code
limit. Fixed-width pushes/targets and64-pair batches plus a tail bound runtime size.
K16 full R2117 is near-full; K4096 full R2 is below99% occupancy because another
uniform round exceeds the charged budget. These points do not establish arbitrary
packing capacity or key-distribution/call-graph bounds.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/transient_storage.py \
  --output-dir "$TRANSIENT_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_transient_storage.py -v
TRANSIENT_STORAGE_NATIVE_BIN="$NATIVE_BIN" \
  TRANSIENT_STORAGE_NATIVE_OUTPUT_DIR="$TRANSIENT_NATIVE_OUTPUT_DIR" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_transient_storage.py -k test_frozen_native_positive_and_dirty_nine -v
```

Use the existing managed Python environment as `PYTHON_BIN`. Catalogue and optional
native-output directories must be fresh. The catalogue binds eight distinct request
hashes, actual generator/imported-source/helper hashes, own-control identities,
frozen model identity and no-fitting provenance. The separate bounded byte VM
checks actual gas, stack/memory/counters, owner-qualified transient values/journal,
frame rollback/exception and transaction finalization. Regression coverage includes
all eight four-user trajectories, full+1 execution under sufficient gas before
constraint rejection, K16 R0/1/63/64/65,100/99-gas TSTORE boundaries, strict API/CLI
and provenance checks, and guard/value/owner/reset/terminal/gas/FD-retention mutants.
The optional native test authenticates the frozen executable, runs exactly a K16
full STOP positive and a native-accepted dirty-interim9 mutant rejected by literal
intent, retains evidence and stops immediately on failure.

Independent exact-source review and all eight canonical native/signature/receipt/
dispatch/authenticated code/account/whole-world/root/witness/original-real-quote/
counter gates precede source/input/tool/resource freeze and sequential SP1 actuals.
The `evm-gas-only` research profile is explicit; it does not establish legacy Unzen
production admission. This new transient eight-input panel stays separate from the
236-input persistent-storage cohort. Compare actual whole-proposal SP1 PGU, literal
node per-block ceilings/sum and the complete-proposal quote separately, preserving
all four own-control increments and four terminal contrasts with exact scaled
deltas and identity flags. Whole-input observations do not allocate actual PGU to
isolated operations. No price/model/Rust/guest/SDK/ELF/pin change or global, proof,
GPU, throughput or production-readiness conclusion follows.

`cases/bn254_pairing_dense.py` adds18 compiler-free BN254 pairing inputs: product
identity or nonidentity, K2/16/64 non-infinity elements, and each group's own
zero/single/generated-full repetition level. Two users call one authenticated
code account with empty persistent storage. Identity inputs use `(jP,Q),(-jP,Q)`
for j1..K/2; nonidentity inputs use `(jP,Q)` for j1..K, whose nonzero exponent
sum yields result0. Both results require successful precompile status1 and valid
user transactions. [EIP-197](https://eips.ethereum.org/EIPS/eip-197) defines the
field/group and192-byte element encoding: G1 x/y then G2 x-imag/x-real/y-imag/y-real.
The producer reuses the existing G1 affine helper; independent test arithmetic
checks G1 schedules and G2 curve membership and subgroup order.

Every level CODECOPYs the same nonempty embedded input, beyond an unreachable
instruction prefix, into memory starting32. The counter occupies word0; output
starts32+192K. Output sentinel2 is allocated once and reset before every
STATICCALL0x08; guards require status1, returndata32 bytes, exact result0/1,
completed counter and STOP. No user persistent or transient storage operations
occur. Empty selected storage support still requires code/account/whole-world
witness authentication; fees, sender nonces and Anchor change whole-world state.

Per call, [EIP-1108](https://eips.ethereum.org/EIPS/eip-1108) native pairing gas is
45000+34000K, plus100 warm STATICCALL access gas. Ordinary gas derives from
executed opcode counts; CODECOPY costs3+18K and memory costs3w+w*w//512 for
w=2+6K. There are no storage refunds or child intrinsic fees. The signed envelope
adds ceil(native/63)+2301 to full gross gas as a conservative fixture reserve;
this is not an SSTORE sentry or a protocol requirement. Zero/single retain their
matching full envelope. The byte oracle verifies actual EIP-150 available,
forwarded and returned gas plus continuation at every call. Full is maximal
uniform repetitions under30M charged block including167355 Anchor,31M header,
every ordered prefix,15M/user fixture cap,16,777,216 EIP-7825 cap and24576 total
code bytes. Occupancy below99% is not near-full; uniform packing is not a global
maximum. Full+1 executes under a sufficient test envelope before candidate
constraints reject it.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/bn254_pairing_dense.py \
  --output-dir "$PAIRING_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_bn254_pairing_dense.py -v
BN254_PAIRING_NATIVE_BIN="$NATIVE_BIN" \
  BN254_PAIRING_NATIVE_OUTPUT_DIR="$PAIRING_NATIVE_OUTPUT_DIR" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_bn254_pairing_dense.py -k test_frozen_native_three_specimens -v
```

Use the managed Python environment; output directories must be fresh. The
catalogue binds18 distinct canonical request hashes and actual source/helper/model
identities. Optional frozen-native smoke runs exactly single/nonidentity K2,
single/identity K64 and a valid reordered K2 input accepted by native pairing but
rejected by literal ordered intent. Every invocation retains command/start/exit/
elapsed/180-second timeout, canonical request, stdout/stderr and source/input/
output hashes, including failures, and stops without implicit retry. The separate
bounded byte oracle validates actual operands before using mathematical product
intent as its precompile result stub; it does not emulate pairing performance.

Independent exact-source review and all18 canonical native/signature/receipt/code/
account/empty-root/witness/operand/wrapper/counter/original-real-quote gates precede
freeze and sequential actual measurements. Compare actual same-proposal SP1 PGU,
literal node block estimates/sum and complete-proposal quotes separately. Retain
all18 raw rows and27 whole-input contrasts: six each zero→single, zero→full and
single→full, plus nine matched identity/nonidentity contrasts. This panel remains
separate from prior persistent236 and transient8 cohorts. No isolated pairing-PGU
allocation, price fit, arbitrary operand/failure/cache bound, global/IID/proof/GPU/
production claim, Rust/guest/SDK/ELF/pin change or deployment follows.

`cases/sha256_bulk.py` generates18 compiler-free SHA-256 precompile0x02 inputs:
zero/a5 tails ×64/4096/1048576 input bytes × own-zero/single/maximal-uniform-full.
Two users call distinct authenticated code accounts with empty persistent storage;
owner tags0/1 and different code hashes are intentional. Call i hashes
`BE256((owner_tag<<128)|i) || pattern_byte*(input_bytes-32)` for i0..R-1.
Every input is unique within its proposal. Equal lengths do not mean equal bytes
or demonstrate general cache behavior.

Both patterns, including R0, use one seed MSTORE and logarithmic MCOPY doubling.
Counter, digest XOR accumulator, input and output start at0,32,64 and64+L.
Memory has3+L/32 words. Output sentinel2 is preallocated and reset before every
call; status1, returndata32, final counter and exact digest XOR guard STOP.
The independent byte oracle checks every ordered input and digest: XOR alone is
not collision-free and repeated output errors can cancel. Empty selected storage
support does not remove code/account/whole-world witness requirements.

Selected host and declared guest revm-precompile43.0.3 `src/hash.rs` and the primary
[SHA-256 execution spec](https://ethereum.github.io/execution-specs/src/ethereum/forks/berlin/vm/precompiled_contracts/sha256.py.html)
charge60+12ceil(L/32) native gas; warm STATICCALL adds100.
[EIP-5656](https://eips.ethereum.org/EIPS/eip-5656) MCOPY costs3+3ceil(n/32),
plus memory expansion3w+w*w//512. Ordinary executed opcodes and21000 intrinsic
are counted with no refund or empty-calldata payload floor. Full is the maximal
uniform repetition count under15M/user fixture cap, EIP-7825's16777216 cap,
31M header and every ordered admission prefix,30M charged block including167355
Anchor, and24576-byte code limit. Full+1 is executed with sufficient test gas
before admission rejection; this is not globally optimal packing. Charged block
gas below29.7M is not near-full. The signed envelope addsceil(native/63)+2301
to full gross as a fixture continuation reserve, with the same envelope for
matching zero/single rows. The oracle checks actual EIP-150 forwarded and returned
gas. Binary search uses fixed-width zero-XOR sizing assembly only; emitted real
guards are computed with stdlib hashlib, independently checked before measurement.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/sha256_bulk.py \
  --output-dir "$SHA256_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_sha256_bulk.py -v
SHA256_BULK_NATIVE_BIN="$NATIVE_BIN" \
  SHA256_BULK_NATIVE_OUTPUT_DIR="$SHA256_NATIVE_OUTPUT_DIR" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_sha256_bulk.py -k test_frozen_native_three_specimens -v
```

Use the existing managed Python environment and fresh output directories. The
catalogue binds18 distinct canonical request hashes and source/helper/model
identities. Only the new focused module is needed; do not discover legacy suites.
The optional Linux native smoke runs exactly a5/1MiB single, zero/64 full and
an a5→5a mutant with recomputed XOR accepted by native but rejected by original
requested intent. Existing `process_guard.py` enforces sampled process-tree PSS
6GiB,180s,256MiB free floor,192MiB output and16MiB retained combined stdout/stderr
log. Each invocation retains canonical request/intent, source bytes/hashes,
command/start/elapsed/exit/limits/outcome, output identities and raw native report;
failures stop without implicit retry. A skipped optional test is not native evidence.

Independent source review and all18 canonical native/signature/receipt/operands/
code/account/empty-root/witness/counter/original-fresh-quote gates precede freeze
and sequential execute-only actuals. Matching smoke positives are reusable only
with an explicit exact source/request/object/GuestInput bridge. Compare actual
same-proposal SP1 PGU, literal node block estimates/sum and complete-proposal
quotes separately. This panel stays separate from prior persistent236,
transient8 and pairing18. No isolated SHA-PGU allocation, price/model fit,
arbitrary-input/failure/cache/global/IID bound, proof/GPU/production parity,
Rust/guest/SDK/ELF/pin change or deployment follows.

The same `cases/sha256_bulk.py` supports `--linear-regime`, selecting
1269664/1269665/2097152-byte inputs in an 18-request matrix with the same two
patterns and zero/single/full levels. The default remains the original 18
64/4096/1048576-byte requests, with frozen canonical request hashes checked by
the focused tests. New lengths use corpus
`unzen-sha256-linear-regime-20261104`; default rows retain their original corpus.
`prepare(..., linear_regime=True)` is the strict keyword-only Boolean selector.
The API accepts only these six named lengths.

For the unaligned 1269665-byte case, output starts at 1269729 and occupies exactly
32 bytes. Memory therefore uses `3+ceil(L/32)` words; the final MCOPY copies
221089 bytes, with gas charged on its rounded word count. Per-call native gas is
476184/476196/786492 for the three selected lengths. In the frozen model's
`max(20M,42*native_gas)` formula, the first two straddle the floor crossover
(19999728 and 20000232); 2MiB gives 33032664. These are formula values, with no
actual PGU or event-attribution claim. The independent oracle checks each input,
digest, unaligned output, copy bounds, counters, EIP-150 forwarding and full+1.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/sha256_bulk.py --linear-regime \
  --output-dir "$SHA256_LINEAR_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_sha256_bulk.py -k Sha256LinearRegimeTests -v
SHA256_BULK_NATIVE_BIN="$NATIVE_BIN" \
  SHA256_BULK_NATIVE_OUTPUT_DIR="$SHA256_LINEAR_NATIVE_OUTPUT_DIR" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_sha256_bulk.py -k test_frozen_native_linear_three_specimens -v
```

The linear native smoke uses the existing guarded runner for exactly
zero/1269664 single, a5/1269665 full and zero/2097152 single, with a 576MiB disk
preflight and the same PSS/time/output/log guards. Its fresh output directory
retains request/intent/row/source and process receipts. Do not rerun the old
native specimens or reuse old measurements under the new source identity.
Matching new positives require exact source/request/GuestInput bridges before
reuse in the canonical panel. The original default panel stays a separate,
read-only research context; new source and helper hashes must be recorded.

### LOG data, topics and receipt-intent bulk probes

`cases/log_receipt_bulk.py` emits30 compiler-free requests in corpus
`unzen-log-receipt-bulk-20261104`: LOG1/LOG4 × five length/pattern domains
(L0/zero, L64/zero/a5, L262144/zero/a5) × own-zero/single/maximal uniform full.
Two users call distinct authenticated, tagged code owners with empty persistent
storage. Each event's topics are ordered
`BE256((tag<<128)|(topic_index<<64)|index)`; nonempty data begins with
`BE256((tag<<128)|index)` and retains the selected tail pattern. Uniqueness is
within a proposal, not across control/single/full observations. L0/a5 is invalid.

Memory counter0 and last-input32 guard loop completion; payload64 allocates
`max(32,L)` bytes once, seeds and logarithmically MCOPYs even for R0. No separate
output buffer, call, precompile, storage access, refund, revert or CREATE occurs.
All successful users terminate STOP. The independent bounded byte VM checks each
ordered address/topic/data tuple, exact source range, gas, copy, counter, terminal
and every opcode count; compact event digests avoid retaining repeated256KiB data.
Its sequence SHA256 is an intent/debug commitment, not a protocol receipt root.

Complete LOG EVM gas is `375+375*N+8*L`, counting the static375 base once: the
selected revm-interpreter43.0.2 host dynamic log cost excludes that separately charged base.
Memory costs `3*w+w*w//512` for `w=2+max(32,L)/32`; MCOPY costs
`3+3*ceil(n/32)` per copy. Empty-calldata intrinsic is21000 and refunds are0.
Full sizing checks the15M fixture transaction cap separately from the16777216
EIP-7825 cap, every ordered31M header admission prefix, and30M charged block gas
including167355 successful Anchor gas. Signed gas is full gross+1000 for every
matched level:1000 is fixture continuation margin, with no CALL/EIP-150 or
SSTORE-sentry rule. Full+1 is independently executed under sufficient gas and
must violate a sizing constraint; uniform full is not global optimal packing,
and near-full>=29.7M is derived per request.

```sh
"$PYTHON_BIN" -S bin/zkgas/bench/cases/log_receipt_bulk.py \
  --output-dir "$LOG_RECEIPT_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_log_receipt_bulk.py -v
LOG_RECEIPT_BULK_NATIVE_BIN="$NATIVE_BIN" \
  LOG_RECEIPT_BULK_NATIVE_OUTPUT_DIR="$LOG_RECEIPT_NATIVE_OUTPUT_DIR" \
  "$PYTHON_BIN" -S -m unittest discover -s bin/zkgas/bench/tests \
  -p test_log_receipt_bulk.py -k test_frozen_native_exact_three_specimens -v
```

Use an existing Python virtual environment and fresh output directories. Export
binds canonical requests, generator, unchanged helper modules and helper source
hashes. The optional Linux smoke invokes exactly3 specimens once: LOG1/L0/zero
full positive, LOG4/L262144/a5 single positive, LOG1/L64/a5 single topic-prefix
intent mutant. The mutant retains gas/routes/loop guards and is natively accepted,
while the independent intent VM rejects its changed topic. The existing process
guard enforces6GiB sampled PSS,180s each,192MiB output,16MiB combined log and a
576MiB disk preflight; retained source/request/intent/row and command/start/caps/
exit/outcome/elapsed/artifact hashes bind every invocation. Skipped smoke tests
are not native evidence. Failures stop without implicit retry.

Frozen `ControlledNativeReceipt` exposes status and gas, not per-log data/topics.
The native constructor compares complete execution results from full and
minimized replay. Byte intent plus native gas/status/routes/replay and later
same-proposal quote/SP1 execution supports this finite panel, without claiming
direct native per-event exports, independently reconstructed receipt roots,
bloom noncollision, per-event tariff exports or per-op PGU. Fresh independent
source review and canonical30 native/60-signature/authentication/world/account/
code/empty-root/witness/byte/receipt/counter/negative/original-quote gates precede
freeze and actuals. Matching smoke positives need explicit source/request/input
bridges. Keep actual SP1 PGU, literal node block-known estimates/sum and complete
proposal estimates separate. Prior236 persistent,8 transient,18 pairing and both
18-case SHA contexts remain read-only, separate cohorts; no price fit, global
bound, ordinary-usability certification or production/proof claim follows.

### Preallocated bulk MCOPY layout probes

`cases/mcopy_bulk.py` emits12 compiler-free requests in corpus
`unzen-mcopy-bulk-20261104`: four layouts × own-zero/single/maximal uniform full.
Every workload copy has length65536. Relative EVM offsets are
same-residue-disjoint64→65600, different-residue-disjoint64→65601,
overlap-right64→65 and overlap-left65→64. These names describe EVM-relative
residues; they do not expose or establish guest heap base alignment. Repeated
disjoint copies may write unchanged bytes after the first copy. Both overlap
directions use snapshot-equivalent copy semantics and evolve each iteration.

Two transactions from one signer with contiguous nonces call distinct tagged,
authenticated code owners with empty persistent storage and empty calldata.
Counter0, protected word32 (`0x7b` repeated32) and payload64 use one preallocation.
`A32=bytes(i|(tag<<7) for i in range(32))`, `B32=inverse(A32)` seed source and
disjoint destination through MSTORE plus logarithmic MCOPY even for R0. Overlap
seeds A plus its first byte; different-residue gap65600 is0xcd; unused padding
remains0xee. Each loop executes exactly one bulk MCOPY and one counter increment.
Final EVM guards check the exact counter and unique valid full words at32,
source/destination head, middle (`base+L//2`) and tail (`base+L-32`), the final
aligned word, plus aligned gap word65600 for different-residue-disjoint. These
sampled guards detect copy-content mutations; the independent VM additionally
checks every prepared/final byte and every copy tuple and content hash. Its
sequence and final-memory SHA256 values are debug intent, not native memory
exports or protocol commitments.

Complete gas includes intrinsic21000, ordinary opcodes including MSTORE8,
`3+3*ceil(n/32)` for each preparation and workload MCOPY, and one memory cost
`3*w+w*w//512`, with refund0. A gas-only fixed-width placeholder assembler sizes
candidates without bulk-byte simulation; selected requests always contain exact
expected guards. Full is the largest uniform repetition count fitting the
separate15M fixture and16777216 EIP-7825 transaction caps, every31M header admission
prefix, and30M charged block budget including successful Anchor167355. All three
levels use that layout's full gross+1000 signed envelope. This1000 is fixture
continuation margin; no CALL/EIP-150 or storage sentry applies. Full+1 independently
executes under sufficient gas and fails sizing. Near-full>=29.7M is derived, and
uniform full is not a global packing bound. Runtime stays below24576 bytes.

```sh
"$PYTHON_BIN" -S -B bin/zkgas/bench/cases/mcopy_bulk.py \
  --output-dir "$MCOPY_CASES_DIR" --model-id "$ZKGAS_MODEL_ID"
"$PYTHON_BIN" -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p test_mcopy_bulk.py -v
MCOPY_BULK_NATIVE_BIN="$NATIVE_BIN" \
  MCOPY_BULK_NATIVE_OUTPUT_DIR="$MCOPY_NATIVE_OUTPUT_DIR" \
  "$PYTHON_BIN" -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p test_mcopy_bulk.py -k test_frozen_native_exact_three_specimens -v
```

Use the existing managed Python virtual environment and fresh output directories.
Export binds requests, generator and unchanged helper module/source hashes.
The optional frozen-native smoke invokes exactly3 specimens once: same-residue-
disjoint/single positive, overlap-right/full positive, and overlap-left/single
intent mutant with loop source65 changed to64 (self-copy), recomputing final
runtime guards so native succeeds while original-intent verification rejects it.
The mutant is not canonical and is never actually sampled. The unchanged runner
uses6GiB sampled process-tree PSS,180s per native,576MiB disk preflight/guard,
192MiB output and16MiB combined log. Each specimen retains request/original-intent/
row/source/helper/native-build identities and command/start/caps/outcome/exit/
elapsed/input/output hashes. Failures stop without implicit retries.

Native exports status/gas/routes; the constructor compares complete execution
results of full and minimized replay. It does not export final memory. Byte
intent and native whole-input binding support this finite panel, without isolated
per-copy PGU, price, proof, GPU timing, production parity or ordinary-usability
claims. Independent source review precedes canonical qualification; the two exact
smoke positives require source/request/input bridges before reuse. Canonical12
require10 fresh native runs,12 fresh original quotes and24-signature/authentication/
world/code/account/empty-root/witness/receipt/byte-intent gates before freeze and
one sequential actual run (four zero, four single, four full). Keep actual SP1 PGU,
literal node block-known estimates/sum and complete-proposal estimates separate.
All prior cohorts remain separate; this producer changes no prices or models.

### Integer division density panel

`cases/division_density.py` exports a finite 32 input panel: DIV/SDIV/MOD/SMOD ×
1/2/3/4-limb divisors × own-zero/maximal uniform full. Each of two distinct tagged
code owners executes a checked warmup and 64 pure arithmetic sites per outer loop,
with retained operand, counter and tag checks. R0 still executes the warmup.
The divisor values are 3, 2^64+1, 2^128+1, 2^192+1; the signed numerator is
-(2^255-1), and the unsigned numerator is 2^256-1. This panel does not establish
that those operands maximize every division implementation branch.

```bash
~/.venv/bin/python -S -B bin/zkgas/bench/cases/division_density.py --output-dir /tmp/division-density-cases
~/.venv/bin/python -S -B -m unittest discover -s bin/zkgas/bench/tests -p test_division_density.py -v
```

The actual-byte oracle independently decodes guards, computes signed/unsigned
math, checks per-PC operands/results and target visits, and accounts every opcode,
memory expansion and ordered gas prefix. Its bounded pure-body cache first proves
the actual decoded 64-site straight-line sequence and complete stack invariant;
it still interprets every outer counter transition. Tests compare direct and
cached execution at R0/1/2 and independently execute both-owner full+1 programs.
Native smoke is opt-in through `DIVISION_DENSITY_NATIVE_BIN` and a fresh
`DIVISION_DENSITY_NATIVE_OUTPUT_DIR`; it launches exactly three guarded specimens,
including a native-accepted divisor 5 mutant rejected by the original divisor 3
intent. Native gas/status/routes and full/minimized replay binding do not export
operand stacks. No actual SP1 PGU, isolated-op cost or price change is inferred.

### KECCAK padding and bulk panel

`cases/keccak_bulk.py` exports27 finite inputs: `(0,zero)` and lengths135,
136,137,65536 with `zero`/`a5`, each at own-zero/single/maximal uniform full.
Two distinct tagged code owners receive empty-calldata calls with signer nonces0/1
and empty persistent storage. Every nonempty loop hashes the unique message
`BE256((tag << 128) | index) || pattern_byte * (L - 32)`. Empty input is explicitly
not unique; its payload nonce word sits outside the hashed range. The existing
EVM empty constant shortcut and model feature `0 if L == 0 else L // 136 + 1`
do not establish actual per-hash PGU. Ethereum Keccak uses suffix0x01/rate136,
not SHA3 suffix0x06; this panel does not allege missing padding charges.

```bash
~/.venv/bin/python -S -B bin/zkgas/bench/cases/keccak_bulk.py --output-dir /tmp/keccak-bulk-cases
~/.venv/bin/python -S -B -m unittest discover -s bin/zkgas/bench/tests -p test_keccak_bulk.py -v
~/.venv/bin/python -S -B -m unittest discover -s bin/zkgas/bench/tests -p test_ethereum_hash.py -v
```

Counter0/digest32/input64 use one aligned allocation. Preparation seeds the entire
extent logarithmically with MCOPY even at R0, restores partial-word padding0xee,
and preserves the explicit preallocation/seed writes. Each loop writes the actual
owner/index nonce, executes KECCAK256 over the declared input, stores the digest,
and advances the counter. Exact final counter, digest, nonce and unique tail-word
guards lead to STOP. Gas-only binary sizing uses fixed-width placeholder guards;
selected runtime source guards use exact Ethereum digests. Every user envelope
is its own full gross gas plus1000, also for zero/single controls. Uniform full
obeys the independent15M fixture/16777216 EIP7825 caps,31M header and every ordered
prefix, and30M charged block budget including successful167355-gas Anchor.

The independent byte interpreter imports the unchanged stdlib Ethereum reference
by an explicit path and a separate module identity. Both producer and oracle
bypass its input cache. Host validation may defer an unread digest using one
opaque token containing an immutable concrete input snapshot. Reads, partial
writes, arithmetic, branching and hash feedback materialize the real reference
value. A provable immediately following full-word overwrite may discard the old
unread token; its actual opcodes remain separately interpreted and gas counted.
The actual EVM program executes every hash. Direct/lazy R0/1/2 differential tests
cover every domain/owner and feedback, partial writes and later input mutation.

Per-user debug hash sequence is
`SHA256(concat(owner_address20 || index_BE8 || offset_BE8 || length_BE8 || SHA256(message)32))`;
R0 commits empty bytes. The loop-entry counter debug sequence is
`SHA256(concat(counter_BE8 for entries 0..R inclusive))`; other reads are separate.
Only the distinct sorted first4/last4 hash-index records are retained (at most8),
with counts and incremental commitments, never every payload or event dictionary.
These are host intent/debug commitments, not protocol roots or native exports.
`expected_last_nonce_word_hex` identifies the payload first word, including L0;
it does not claim that empty hash input contains that nonce or is unique.

Catalogs declare `source_root='bin/zkgas/bench'`; dependency hashes and native
source snapshots use normalized benchmark-relative `cases/...` and
`tests/ethereum_hash.py` paths. The maintained reference is a benchmark dependency,
never guest/production code. Native smoke is opt-in with `KECCAK_BULK_NATIVE_BIN`
and a fresh `KECCAK_BULK_NATIVE_OUTPUT_DIR`. Exactly three specimens launch once:
L0/zero/full, L65536/a5/single, and an L136/a5/single nonce-prefix-bit80 variant
whose exact digest/nonce guards are recomputed. The variant succeeds natively
but is rejected by unchanged original intent; it is never canonically sampled.
The unchanged process guard retains source/helper/request/intent/row/build/input/
output identities and command/start/outcome under6GiB PSS/180s/576MiB disk/
192MiB output/16MiB log limits. Native gas/status/routes and full/minimized replay
binding do not export per-call memory/input/digest. Anchor may also hash.

Independent source/behavior gates precede canonical27 qualification and freezing.
The two exact smoke positives need source/request/input bridges before reuse;
remaining25 native inputs,27 fresh original quotes and54 signature/authentication/
world/code/account/empty-root/witness/receipt/byte-intent gates precede actual27
once (nine zero/nine single/nine full). Actual SP1 PGU, literal block-known node
estimates/sum and complete-proposal estimates remain separate. All previous
cohorts stay separate; no isolated-op cost, price/model change or proof/production
claim follows from this generator.

### Full-width modular arithmetic density

`cases/modular_density.py --output-dir <fresh-directory> [--model-id <sha256>]`
exports16 compiler-free ADDMOD/MULMOD inputs: four moduli (3,2^64+1,2^128+1,
2^192+1), each own-zero/full. Both operands stay fixed at2^256-2 and2^256-3.
Their exact257/512-bit intermediate reductions differ from a premature256-bit
wrap in all eight operation/modulus domains. This finite panel does not assert
worst-case modular operands or quotient correction behavior.

Each proposal has two successful user calls to distinct tagged code owners with
empty persistent storage/calldata. A warmup executes even at R0. The64-site body
uses `DUP3 DUP3 DUP3 ADDMOD|MULMOD POP`, preserving `[modulus,b,a]`; counter, tag
and all three retained operands are checked before an empty-stack STOP. Tags
change code identities; target arithmetic inputs are constant. Target count is
64R+1 per user and each pure site costs19gas. The separately counted loop uses
ordinary ADD. Gas sizing uses assembled instruction multiplicities without
replaying full bodies. Uniform full is maximal under15M fixture and16777216
EIP7825 envelopes,31M header admission prefixes and30M charged block including
Anchor167355; zero and full share full gross gas plus1000 as their envelope. Full+1 is
checked for both owners in every domain. These limits do not establish globally
optimal packing.

Run focused checks with the configured Python interpreter:

```sh
"$PYTHON_BIN" -S -B -m unittest discover -s bin/zkgas/bench/tests -p test_modular_density.py -v
```

The separate `tests/modular_density_vm.py` exports `execute` and `run_users`.
It derives gas/routes from actual decoded bytes and checks ordered operands,
results, guards and every surrounding instruction. Its optional host cache
proves the exact64-site pure stack invariant and is compared with direct R0/1/2
execution. Actual guest execution performs every target. Catalog dependency
keys are benchmark-relative `cases/...` under `source_root='bin/zkgas/bench'`;
existing helper sources and source snippets are hash-bound and unchanged.

Optional frozen-native smoke uses `ZKGAS_FIXTURE_BIN` and a fresh
`MODULAR_DENSITY_NATIVE_OUTPUT_DIR`; run only
`test_modular_density.ModularDensityTests.test_frozen_native_exact_three_specimens`
with `bin/zkgas/bench/tests` on `PYTHONPATH`. Exactly three guarded specimens
launch once: MULMOD/limb3/full, ADDMOD/limb1/zero, and MULMOD/limb1/zero whose
actual modulus is5 while preserved original intent requests3. Both moduli
produce the correct result2, so the accepted mutant must fail ordered intent
validation beyond output equality; it is excluded from canonical measurements.
Source/helper/request/row/original-intent/build/input/output and process receipts
are retained under6GiB sampled PSS/180s/576MiB free disk/192MiB output/16MiB log
limits. Free disk576MiB comprises256MiB runtime plus320MiB retention reserve.
Native gas/status/routes, full/minimized execution-result equality and same-input
stage validation do not directly export arithmetic operands, stacks, results or
per-op PGU. Independent canonical16 qualification, fresh quotes and source/input
freeze precede actual16. No pricing, model, proof or production claim follows.


## Bounded cold code reads

`cases/cold_code_reads.py` emits12 compiler-free original fixtures:
BALANCE (`0x31`) or EXTCODESIZE (`0x3b`), configured footprints16/128/512,
and each group's own `zero`/`selected` level. Each fixture has one signed
EIP-1559 user CALL (nonce0, empty calldata/value0) to the primary program.
Every level retains the same configured target count. Selected visits each
consecutive target once; zero visits none. Target accounts have zero balance,
empty persistent storage and unique24576-byte legacy runtime
`PUSH32(index+1) || STOP || JUMPDEST*24542`. Fixture predeployment occurs outside
the measured block and is not free over the lifecycle. Targets are read without
CALL, DELEGATECALL, creation, logs, persistent writes or refunds.

```sh
python -S -B bin/zkgas/bench/cases/cold_code_reads.py \
  --output-dir artifacts/cold-code-reads/recipes --model-id "$FROZEN_MODEL_ID"
python -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p test_cold_code_reads.py -v
# Optional: existing frozen binary and fresh retention root; runs exactly3 once.
ZKGAS_FIXTURE_BIN=/path/to/frozen/native \
COLD_CODE_READS_NATIVE_OUTPUT_DIR=artifacts/cold-code-reads/native-smoke \
  python -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p test_cold_code_reads.py -k test_frozen_native_exact_three_specimens -v
```

The maintained APIs are `make_case(operation='size', footprint=128,
level='selected')`, `_runtime(operation, footprint, visits)`,
`gas_accounting(operation, footprint, visits)` and
`prepare(output: Path, *, model_id=MODEL)`. Public levels are `zero` and
`selected`; direct bounded byte tests also cover0/1/2 visits. Exports stream one
request at a time into a fresh output directory and retain request, target-code,
source/helper SHA256 commitments. Metadata stores no duplicate target code blobs.

The runtime checks every result (BALANCE0 or size24576) and final counter. A
separate bounded byte oracle checks counter progression and clean terminal stack,
and records actual decoded opcode counts, ordered address/value SHA256, cold set,
counter commitment and at most eight samples. These commitments are host intent/debug
evidence, not protocol roots or direct native per-read exports. Under the frozen
pre-Amsterdam domain each target costs2600 cold-account gas; witness preloading
does not warm access. Ordinary/memory gas,21000 intrinsic and167355 Anchor gas
are separate. Each footprint retains one envelope across both operations/levels,
max selected gross+1000, checked against15M fixture,16777216 EIP-7825,31M header
prefix and30M charged-block limits. K512 is a fixture bound, not a protocol limit
or a gas maximum; this is an unsaturated selected-footprint pilot.

The opt-in native smoke verifies the existing executable/build/source identities,
then runs sizeK16 selected and balanceK128 selected originals plus a same-length
sizeK16 target-tail mutant exactly once. Native accepts the mutant's length
guards while original content/SHA256 intent rejects it. Originals alone are
eligible for exact canonical bridges. Each run retains source/helper/request/row/
original-intent/native-build hashes, prelaunch files, command/start/caps/outcome,
canonical input, public output and raw report. The unchanged guard enforces6GiB
sampled PSS,180 seconds,576MiB disk headroom,16MiB log and192MiB combined growing
outputs; uncompressed32MiB request,128MiB canonical/native and64MiB benchmark
input limits remain. Failures stop without implicit retry.

Qualification requires target-code SHA256 intersection with minimized witness
bodies: size-selected needs every visited target body, while BALANCE and zero
need none. Account proofs, primary/system code and whole-world commitments remain
separate. Independent canonical/signature/genesis/account/code/root/receipt/quote
verification still precedes actuals. Full target loading in a declared source
path with an Account-only node selector does not establish underpricing:
EXTCODESIZE already reserves1M per event plus64 dispatch and code64 PGU/byte;
whole-proposal pricing and literal node block-known estimates remain distinct.
No price, model, SDK, guest, ELF, dependency or build change follows.

The two dependency wording corrections above identify selected host/declared
guest sources, not an ELF build attestation. The relevant41/43 function bodies
and formulas were verified unchanged; historical source hashes and data remain.

### Fixed-operand EXP density and popcount panel

`cases/exp_density.py` exports10 finite holdout inputs: five exponent classes
`dense2=2^16-1`, `sparse24=2^191`, `dense24=2^192-1`, `dense25=2^200-1`,
`dense32=2^256-1`, each at own `zero` and `full`. The common base is the odd
full-width word `0xa5` repeated32bytes. Sparse24 and dense24 have the same
bit length192,24 exponent bytes and EVM gas, with popcounts1 and192. Dense25
crosses the selected node floor/curve transition. These finite inputs do not
establish worst-case bases, compiled carry paths or global bounds.

The maintained APIs are `_operands(exponent_class)`,
`_runtime(exponent_class, repetitions, tag=0)`,
`gas_accounting(exponent_class, repetitions)`, `fits`, `full_repetitions`,
`make_case(exponent_class='dense24', level='full')` and
`prepare(output: Path, *, model_id=MODEL)`. The CLI accepts only `--output-dir`
and `--model-id`. Exports stream requests into a fresh directory, pop request
bodies from catalog rows and retain generator, five source-module and helper
snippet SHA256 identities under normalized `bin/zkgas/bench` paths.

Two signed user transactions, nonces0/1 from the existing constructor, visit two
distinct tagged deployed code owners with empty persistent storage. Fixed-width
PUSH32 exponent then base produces retained stack `[E,B]`: selected revm43 EXP
pops the top base and exponent below. One guarded warmup remains at R0; each
loop executes64 `DUP2 DUP2 EXP POP` sites that restore both original operands.
Final operand, memory-counter and owner-tag guards precede empty-stack STOP;
failed guards reach INVALID. Every EVM/SP1 site executes normally. There is no
result chaining, deployment, call, storage, log, calldata or refund work in the
body. Predeployment setup is outside the measured block and has a lifecycle cost.

The independent `tests/exp_density_vm.py` decodes actual bytes without producer
imports, checks fixed widths, control targets, ordered operands, modular256
results and gas, and derives counts/counter/tag/terminal state. Its optional
pure-body cache proves all64 decoded sites on the complete stack and checks
that same complete stack at every body entry; outer memory, counter and control
flow remain direct. Focused tests compare direct and cached R0/1/2 for all five
classes/both tags and reject feedback, swapped/omitted operands or EXP, malformed
sites, altered immediates, failure targets, owners and schedules. Semantic
corners cover exponents0/1/127/128/255/256/511 and bases0/1/2/odd-wide, including0^0.
No full instruction trace or per-step runtime list is exported.

EXP gas is exactly `10+50*ceil(bit_length(E)/8)`, including static10 once.
Actual byte execution independently rederives intrinsic21000, memory6 and
ordinary gas. Full selects maximal uniform64-site batches under30M charged
block gas including Anchor167355 and two users,15M signed envelopes,16777216
EIP-7825,31M header ordered-prefix admission and runtime size. The full envelope
is full user gross+1000 and remains in own-zero. Each full+1 fails the charged
block budget; this is distinct from unrestricted packing. `near_full` is derived
at99 percent. Source `h+popcount(E)` wrapping multiplication counts describe the
selected ruint algorithm, not actual PGU or a node pricing selector.

The opt-in focused native test runs exactly three specimens once: dense32-zero
and dense24-full originals, then a dense32-zero exponent-intent mutant. For one
owner only, the mutant changes MASK to MASK-2^254 in exactly two decoded PUSH32
contexts (initial exponent and final retained-exponent guard). Exact modular
results, bit length and event gas are equal; native accepts while original byte
and content intent rejects it. The original row/request remain retained, and
the mutant is excluded from canonical/actual inputs. Only the two originals
can bridge to canonical qualification through exact source/helper/request/row/
input/public-output identities. Native gas/status/routes and full/minimized
execution-result replay do not export per-EXP operands, results or stacks.

Each native run retains command/start/outcome, immutable requested intent,
source/helper/build identities, caps, raw report, canonical input and public
output. Unchanged guards enforce6GiB sampled tree PSS,180 seconds,16MiB logs,
192MiB growing outputs and576MiB disk headroom (256MiB plus320MiB retention);
request32MiB, native128MiB and benchmark input64MiB caps remain. Failures stop
without retry. No source-author quote, SP1 run or build is part of this smoke.
Independent signatures, receipts, code/accounts/roots/empty selected persistent
support, witness and original quote gates precede actuals; source counts alone
do not establish a PGU deficit, per-operation cost or pricing defect.

### Dense in-bounds word memory

`cases/memory_word_density.py` prepares the finite12-input
`unzen-memory-word-density-20261104` panel: MSTORE at offsets0/1 in256-byte and
1MiB arenas, plus MLOAD at offset1 in both arenas, each own-zero/full. Two signed
empty-calldata CALLs visit distinct tagged predeployed owners with empty persistent
storage and contiguous signer nonces0/1. Predeployment is outside the measured
block. The generator assigns no PGU, model coefficient or pricing change.

Each runtime expands exactly once with `MSTORE8(arena_bytes-1,0)`. Counter64 and
tag96 are disjoint from target bytes0..32 and the final arena word. Store retains
`[A,B,offset]`, where A=`a5` repeated32 bytes and B=`5a` repeated32 bytes. It warms
up and checks both values, then executes32 alternating A/B pairs per64-site batch:
`DUP3 DUP2 MSTORE`, `DUP2 DUP2 MSTORE`. Load seeds B, checks one warmup, retains
`[offset]` and executes64 `DUP1 MLOAD POP` sites per batch. Final checks bind B,
zero last word, original retained operands/offset, counter and owner tag before
empty-stack STOP; failure reaches INVALID. Every actual EVM/SP1 target executes.

Targets cost3 EVM gas each; all DUP/POP/control/setup/guard instructions are counted
separately from exact memory expansion `3W+floor(W²/512)`, W=`arena_bytes/32`.
Metadata distinguishes target warmup/body events from counter/setup/guard uses of
the same opcode. Full is the maximal uniform64-site batch count under30M charged
block gas, the15M fixture signed-envelope cap,31M ordered header prefixes,
EIP-7825 and runtime limits; it is not unrestricted packing. Own-zero keeps the
full signed envelope. Full+1 identifies the failing constraints, and `near_full`
is derived independently at99 percent.

```bash
python -S -B bin/zkgas/bench/cases/memory_word_density.py --output-dir artifacts/memory-word-density
python -S -B -m unittest discover -s bin/zkgas/bench/tests -p test_memory_word_density.py -v
```

Use the existing managed virtual environment's interpreter. Fresh output paths
are mandatory. Requests stream to individual files; catalog rows retain metadata
and hashes. `source_root` is normalized to `bin/zkgas/bench`; unchanged helper
module/snippet hashes accompany the generator hash.

`tests/memory_word_density_vm.py` imports no producer. It decodes the complete
bytecode, models a bounded bytearray and checks both owners, exact stack/addresses/
values, expansion, gas, counter progression and route counts. Its optional host
body cache first proves the complete decoded stack and ordered read/write
footprint:64 alternating stores at the actual fixed address ending B, or64 B reads
with unchanged memory. Each cached application checks live full-stack and target
memory preconditions and applies that proven stateful transition. No outside
writes or memory growth are permitted; outer flow, counter, tag and guards execute
directly. Direct R0/1/2 comparisons and memory-feedback, changed-value/offset,
omitted-read/write, alias, counter and tag mutations exercise this boundary.

Optional frozen-native smoke uses `ZKGAS_FIXTURE_BIN`, a fresh
`MEMORY_WORD_DENSITY_NATIVE_OUTPUT_DIR`, and
`test_memory_word_density.MemoryWordDensityTests.test_frozen_native_exact_three_specimens`
with `bin/zkgas/bench/tests` on `PYTHONPATH`. It invokes exactly three guarded
specimens once:1MiB offset1 store-full original,256-byte offset1 load-zero original,
and the same store-full with one owner's A changed to `aa` repeated32 bytes in
exactly three decoded contexts (initial retained A, warmup-A expected guard, final
retained-A guard). Decoded widths and context count are checked before launch.
The changed-A program remains internally consistent and native-accepted, while
original A/request/row intent rejects it; B/final memory, gas and routes remain
unchanged. Only the two originals are eligible for exact canonical bridges.

Each native run retains command/start/outcome, source/helper/native/build/input/
request/row/original-intent identities, public output, routes, receipts and resource
records. Unchanged guards enforce6GiB sampled tree PSS,180 seconds,16MiB logs,
192MiB staged output and576MiB preflight disk reserve. Request32MiB, canonical/native
128MiB and benchmark-input64MiB caps remain. The1MiB arena is runtime memory, not
an encoded megabyte payload. First failures remain retained; no hidden retry.
Native status/gas/routes and whole-input replay do not directly export final memory,
per-target values, actual pointers or memcpy branches. Selected source/ELF alignment
classes do not establish per-occurrence execution, an MLOAD8x multiplier, isolated
word PGU, global bounds, proof/GPU behavior or production readiness.

### Empty-storage warm-read calldata catalogs

`cases/warm_read_catalogs.py --output-dir <fresh-directory>` exports four finite
inputs: deterministic ordered and SplitMix64 catalogs, each with its own zero and
maximal single-user full level. Each catalog has512 unique keys, zero low192 bits,
nonzero bytes in high64, and fixed final query `0x1122334455667788 << 192`.
The runtime depends only on the repetition count; keys are the existing native
`program.input_hex` calldata. The default primary has empty persistent storage.
There are512 cold zero reads followed by a guarded warm query and64 warm reads
per batch. The independent byte oracle checks calldata order, complete stack,
warmness, zero values, counters, sentinel, gas and empty-stack STOP. Its proven
pure-read body cache is test-only; actual EVM execution performs every read.

The16384-byte calldata contains12288 zero and4096 nonzero bytes. Ordinary
intrinsic135688 and EIP-7623 floor307720 are separate; the floor is inactive after
the cold reads. Full is maximal uniform R under a15M single-user envelope, with
EIP-7825,31M header prefix and30M charged block limits checked separately. This
single-user pilot does not certify a full30M block or equal guest hash-map seeds.
Signatures and input-dependent earlier allocation can differ across catalogs.

Optional `--catalog-file <json-file>` exports only two custom development probes.
The bounded128KiB JSON must contain exactly `schema_version: 1` and512 canonical
lowercase U256 strings in `keys`, with512 unique values and the same fixed query
last. Each custom key must contain exactly eight nonzero bytes; their positions
may vary. This preserves calldata gas while permitting short comparison prefixes.
Default ordered/mixed catalogs retain their original high64 placement and bytes.
Custom IDs use the full canonical calldata SHA256; JSON, source and calldata
hashes are retained without local filenames or paths. Keep data-only catalogs in
Atlas. No model fitting, pricing, seed search or collision claim is performed.

With an external catalog, optional `--probe-repetitions 512` exports three rows in
zero/probe/full order. The probe must satisfy `1 <= R < full_repetitions`; zero,
the duplicate full endpoint and use without `--catalog-file` fail before output
creation. `make_probe_case(keys, repetitions)` exposes the same constructor API.
All three retain the original full transaction gas envelope; the probe's code,
gas, counters, occupancy and actual64R sample ID describe its own execution. It is
a development probe with its own zero baseline, with no maximal-gas claim.

`hash_probe_catalog.rs` is a standalone std-only data generator. Build it directly:

```bash
rustc --edition=2024 -O bin/zkgas/bench/hash_probe_catalog.rs -o experiment/hash-probe-catalog
experiment/hash-probe-catalog hash --seed 0x0000000000000000 \
  --shared 0x0000000000000000 \
  --key 0x1122334455667788000000000000000000000000000000000000000000000000
```

Hash accepts1..1024 repeated `--key` arguments, returns canonical keys and full
U64 hashes in input order, and treats k0 as the least significant U64 limb of the
canonical U256. Scalars use lowercase `0x` plus16 hex digits; keys use64 digits.
It computes `F(x,y)=low64(x*y) XOR high64(x*y)` with a full U128 product,
`A=F(k0 XOR S,k1 XOR G)`, then `H=F(k2 XOR A,k3 XOR G)`.

For a bounded search, supply explicit hypothesized `--seed` and `--shared` plus
`--query <canonical-U256> --bucket-mask <mask> --max-trials <N> --max-seconds <N>
--output <fresh-file>` after `search`. The query requires zero low192 bits and
eight nonzero high64 bytes. A deterministic U32 permutation varies only its high32
bits by default (`--domain high32`). With `--domain low32`, candidate words are
placed in the least-significant limb and their low32 bits vary; the query keeps
its original high64 placement. Both domains retain exactly eight nonzero bytes
per key,511 unique candidates matching both `H & mask` and `H >> 57`, and the
fixed query last. The low32 domain probes short key-comparison prefixes without
changing runtime or calldata gas. Matching this hypothesized profile does not
establish the actual guest seed or workload; verify realization during replay.
The warm-read constructor requires its documented fixed query.
Masks are positive powers of two minus one through65535; bounds are1..268435456
trials and1..120 seconds. A monotonic deadline is checked every1024 trials and at
completion. Callers must also use the existing external process/resource guard.

Exit0 atomically installs the strict data-only catalog without replacing a file
or symlink, including concurrent creation. Exit2 reports an incomplete search and
creates no catalog; exit1 reports invalid input or an installation/I/O error.
Search stdout is bounded metadata with exact trials, integer elapsed nanoseconds,
parameters and status; `found` includes the fixed query, while
`retained_candidates` excludes it. Metadata contains no output path. Hash JSON,
search metadata and catalog limits are256KiB,16KiB and128KiB respectively.
Same-directory owned temporary files are cleaned on ordinary error/exit; a killed
process can leave a bounded temporary file, so retain and inspect its outcome.
Freeze source, compiler, executable, arguments and output SHA256 identities.

Synthetic tests do not establish actual hash agreement or new-input seed
stability. Independently validate all512 captured hashes before real search,
then observe the new signed zero input's own seed and cold hashes before larger
actuals. Concentrating a bucket/control tag does not assert full-hash collisions,
occupancy, underpricing, or proof/production parity.

Focused validation uses the managed Python environment:
`python -S -B -m unittest discover -s bin/zkgas/bench/tests -p 'test_*catalog*.py' -v`.
This builds the standalone helper directly with rustc and uses synthetic hash
operands plus the independent byte oracle. Its Rust unit tests can also run via
`rustc --edition=2024 --test bin/zkgas/bench/hash_probe_catalog.rs -o <temporary-test-binary>`.
The optional frozen native gate requires `ZKGAS_FIXTURE_BIN` and a fresh
`WARM_READ_CATALOGS_NATIVE_OUTPUT_DIR`; it runs exactly three specimens: ordered
full, mixed zero, and a same-gas first-two-key order mutant. The mutant retains
original catalog intent, fails that intent gate, and is excluded from canonical
inputs. Native reports status/gas/routes and replay output, not per-read keys or
values; primary empty-root/code support is authenticated separately from the
configured extra-contract list.

## Selected-PC historical register diagnostics

The standalone estimator optionally accepts `--observation-plan plan.json`; see its
[plan and report contract](../sp1-estimator/README.md). Omission preserves the ordinary
measurement contract. Observation plans are ELF-bound, strict and bounded; no storage PC,
seed or map assumption is built into the tool. Run diagnostics separately from original
pricing actuals and retain their new source/executable identity. Diagnostic wall time
includes public SDK instruction replay and must not be treated as prover time.

The canonical producer, GasVM, two raw slots, memory-entry chunk threshold, stdin framing,
resource receipt and gas/public-value fields remain authoritative. Sample truncation and
instruction-limit/error status remain explicit. An observed register is an actually
accessed historical value, not an arbitrary RAM or whole-table snapshot. Qualify the tool
against frozen default/observed controls and source-expected PC counts under the existing
PSS/time/log/staging/disk guards before using values to identify a hasher or map.

### Absent-account journal catalogs

`cases/account_read_catalogs.py` is a separate Address20 family. It does not change the existing
storage catalogs, U256 hash-helper defaults, guest, or pricing. The initial export is one BALANCE
own-zero plus one 512-repetition probe; `make_case(operation='extcodesize', ...)` exposes the same
explicit byte/gas contract without expanding the default experiment matrix. Both are opened
**development probes**, not IID/holdout data or actual PGU evidence. Model
`5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11` and 384 storage PGU/unit remain
frozen. The generator records an intended model identity; the separately selected quote executable
must independently report that exact model.

```sh
~/.venv/bin/python -S -B bin/zkgas/bench/cases/account_read_catalogs.py \
  --output-dir experiment/account-default --probe-repetitions 512
~/.venv/bin/python -S -B bin/zkgas/bench/cases/account_read_catalogs.py \
  --output-dir experiment/account-custom --catalog-file account-keys.json --probe-repetitions 512
~/.venv/bin/python -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p 'test_account_read_catalogs.py' -v
```

An external data-only catalog has exactly `{"schema_version":1,"keys":[...]}`. It must contain 512
unique canonical lowercase 20-byte `0x` addresses, each with eight nonzero bytes, with fixed query
`0x1122334400000000000000000000000055667788` last. Reserved fixture/system addresses and the low
precompile address range are excluded. Calldata contains left-zero-padded 32-byte ABI words, not
U256 storage keys or packed 20-byte records: 16,384 bytes, including 4096 nonzero bytes. The default
catalog uses deterministic mixed nonzero four-byte suffixes after `11223344` plus twelve zero bytes.
A shared raw prefix alone does not establish actual hash-table concentration; external short-prefix
catalogs preserve calldata byte cost and runtime at the same operation/repetition count.

The opt-in `make_sized_case(keys, *, operation='balance', repetitions=512,
access_list=False)` accepts 1..1024 unique canonical lowercase Address20 strings and uses
`keys[-1]` as its query. It permits arbitrary nonzero-byte counts, while `make_case`, `prepare`
and the CLI retain their strict512/eight-nonzero/fixed-query byte contract. For example, with the
cases directory on `PYTHONPATH` and an explicit ordered `addresses` list:

```python
from account_read_catalogs import make_sized_case

case = make_sized_case(addresses, repetitions=1024, access_list=True)
request = case['request']  # ordinary ProgramRequest for the existing workload harness
```

The sized path keeps the same calldata seed and 64-read memory-counter loop, one CALL user,
31M header, fixed15M transaction envelope and no prestate targets. `access_list=True` declares
exactly the same ordered addresses as raw empty-key items. Every seed is then warm; subsequent
queries remain warm in either mode. Without AL there are N cold seeds plus 64R+1 warm queries;
with AL there are N+64R+1 warm reads and no cold seed. AL declaration does not itself materialize
the journal Account table or prove any map size, own seed or hash pressure.

Gas uses actual ABI bytes: let I=21000+4*zero_bytes+16*nonzero_bytes. Gross user gas is
I+2692N+359+6775R without AL, or I+2592N+359+6775R with AL (including 2400/item intrinsic).
The separately recorded EIP7623 floor is21000+10*(zero_bytes+4*nonzero_bytes); charged gas is
max(gross,floor), with no refund. Over-budget inputs fail before returning a row. The sized
metadata includes `sized_account_catalog`, `key_count` and `access_list`, and does not claim
legacy full-R/headroom bounds. It selects no pricing model or observed G and exports no input.
The independent decoded-byte VM, signed-input/absence authentication, current quote and any
actual/profile gates remain separate requirements before using a generated case as evidence.

The standalone helper defaults to U256; select `--key-kind address20` explicitly for this family.
Using the helper built above, hash an address or search a fresh catalog with the **observed**
account map's seed, shared word and mask:

```sh
experiment/hash-probe-catalog hash --key-kind address20 \
  --seed "$ACCOUNT_SEED" --shared "$ACCOUNT_SHARED" \
  --key 0x1122334400000000000000000000000055667788
experiment/hash-probe-catalog search --key-kind address20 --domain low32 \
  --seed "$ACCOUNT_SEED" --shared "$ACCOUNT_SHARED" --bucket-mask "$ACCOUNT_MASK" \
  --query 0x1122334400000000000000000000000055667788 \
  --max-trials 268435456 --max-seconds 120 --output experiment/account-keys.json
```

Seed/shared scalars require `0x` plus 16 lowercase hex digits. For Address20, `low32` varies the
last four network-order address bytes and keeps the first 16 identical; `high32` varies the first
four bytes and keeps the last 16 identical. Address comparison starts at network byte zero, so
these are long- and short-comparison-prefix controls respectively. This differs from U256's
little-endian limb layout. A search match remains conditional on its supplied profile; independently
observe the new input's actual map, seed, hash, mask and coverage before claiming concentration.

The fixed bytecode performs 512 ordered cold account reads, one warm query read, and 64 warm query
reads per repetition. Zero-result, exact calldata length, counter, sentinel, query and empty-stack
checks guard successful STOP. The decoded-byte oracle independently computes execution, addresses,
cold/warm state and gas without calling producer runtime/gas functions. It is authored alongside
the generator and does not itself constitute independent authorship or authenticated account absence.
Native GuestInput/root/proof checks remain required before actual measurements.

Targets are absent accounts with no provisioned code or storage. Requests omit `prestate_contracts`,
`storage_state`, access lists and authorizations, using one CALL to the normal primary runtime.
The existing constructor's complete tiny base-state trie proves these absences without adding 512
accounts or dummy target code. Its canonical honest witness is not claimed to be a globally minimal
node subset; the native-read multiproof minimizer is not invoked on this constructor path.

For BALANCE and EXTCODESIZE under the fixed post-Unzen profile, cold reads cost 2600 gas and warm
reads 100. The R512 probe predicts 4,983,151 user receipt gas plus 167,355 Anchor gas (5,150,506 block
gas); own-zero predicts 1,514,351 user gas. Both retain the common 14,997,601 transaction gas envelope,
which derives from the single-user maximum R1990 plus 1000 headroom, below the 15M fixture cap and
EIP7825 cap. The header remains 31M, with 30,832,645 gas remaining after Anchor. The additional 30M
charged-block experiment budget is separate. A single-user 15M envelope is not a full 30M block packing
certificate. Calldata intrinsic gas is 135,688; the 307,720 EIP7623 floor is retained separately.

`catalog.json` binds generator/imported assembler source hashes, helper source hashes, exact request
hashes and external catalog hash, while excluding local catalog filenames. `make_case` supports
`zero` and intermediate `probe` levels; `prepare` exports exactly those two BALANCE rows. Invalid
catalogs/repetitions/model IDs fail before creating the output directory, and existing outputs are
never replaced. External catalog input is bounded to 128KiB and distinguishes Address20 from U256.

A frozen constructor can consume a selected emitted request without new Rust/guest construction.
Set `ZKGAS_FIXTURE_BIN` to the independently verified executable, and use distinct fresh paths:

```sh
"$ZKGAS_FIXTURE_BIN" --execution-profile evm-gas-only \
  --input selected-request.json --output experiment/input.json \
  --output-bincode experiment/input.bin --report experiment/native.json
```

Preserve source/binary/input/options identities and all rejected/unrealized pressure cases. Confirm
native signatures, account absence, roots, route counts, receipt gas and canonical output before
comparing node-known quote, completed-proposal quote and same-input SP1 PGU. This family changes
neither prices nor deployment policy; any subsequent price decision requires actual deficit evidence.

### Explicit storage-catalog capacity probes

`warm_read_catalogs.py` accepts `--key-count K`, an integer from1 through1024
(default512). The Python factories `make_case`, `make_custom_case`,
`make_probe_case`, gas helpers and `prepare` accept the same keyword-only
`key_count=512`. Existing default512 requests, IDs, runtime and envelopes retain
their original bytes. Nondefault rows carry `parameters.key_count` and `-kK-`
in their sample IDs; the byte oracle authenticates K against calldata, decoded
runtime, executed reads and metadata. Missing new-count metadata uses the legacy
512 shape and therefore cannot silently accept another calldata length.

For an external data-only catalog containing exactly K canonical unique U256
keys (eight nonzero bytes each, unchanged query last), export to a fresh directory:

```bash
"$PYTHON_BIN" -S -B bin/zkgas/bench/cases/warm_read_catalogs.py \
  --output-dir experiment/storage-capacity-k449 \
  --catalog-file experiment/catalog-k449.json --key-count 449 \
  --probe-repetitions 512 \
  --model-id 5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11
```

The existing export contract produces zero/probe/full rows; select zero and probe
for a fixed-R panel, retaining the same K-specific full envelope. Catalog JSON
remains capped at128KiB and existing output paths are rejected. The four selected
K448/449/896/897 controls do not establish a capacity transition: observe the exact
guest table mask before making that claim. Use deterministic low32-diverse
external catalogs to isolate growth from prefix concentration, without hash seed,
bucket or tag search. Default ordered/mixed catalogs retain zero low192 bits and
are not that baseline. No pricing or account-journal correction is performed.

For this finite zero-state workload, calldata is32K bytes (24K zero,8K nonzero),
intrinsic gas is21000+224K, the EIP-7623 floor is21000+560K, and cold/warm SLOAD
fees are2100K and100(64R+1). Including guarded-loop ordinary gas and12 memory gas,
gross gas is21359+2416K+6775R; charged gas is the larger of gross and the floor.
There are no SSTOREs or refunds. Existing15M fixture envelope, EIP-7825,31M header
prefix and30M charged-block checks still apply. Rollback/refill and a30M full-block
packing experiment are separate follow-ups.

### Explicit rollback-capacity keys and user counts

`storage_dirty_keys` and `storage_warm_rollback` accept explicit strict integer
body keys1..1024; their historical default export grids remain1/16/256/1024 and
16/256. The warm family adds keyword-only `user_count=4` (strict1..4) to
`make_case`, `fits`, `full_rounds` and `prepare`. Dirty keeps its fixed four-user
API. Existing default warm32 and dirty16 complete rows and requests retain their
original bytes. Nondefault warm F adds `parameters.user_count` and `-uF-` to IDs.

Only selected owners/chains are provisioned: F top storage owners, F wrappers at
each delegated level and one shared leaf code account. Depth2/F2 therefore has
five reachable configured contracts and two signed user CALLs. Their phase slot0
plus K body slots means logical map sizes K+1; no discarded F4 chain/code/storage
or aggregate statistic is retained. Full-R selection and signed full envelope
are recomputed for F, including every ordered header prefix. Per-user EIP-150,
12M child request,2301 leaf reserve,15M fixture and EIP-7825 limits are unchanged.

For a finite R0 matrix, use the existing factory directly:

```python
from storage_warm_rollback import make_case

rows = [make_case("leaf", keys, 2, prewarm, "zero", user_count=2)
        for keys in (447, 448, 895, 896) for prewarm in (False, True)]
```

R0 executes real seed1→5, cleanup5→1, lower REVERT and parent postchecks; it is
not empty execution. Top transactions succeed, effective/applied refunds are0,
and original values are restored. Caller prewarm survives the lower checkpoint;
without it, child-added warmth is rolled back before the cold parent postcheck.
Loaded physical journal entries are a separate observation: this family does not
claim that rollback clears/rebuilds the physical table or covers clear/refill.

`prepare(..., key_counts=(447, 448, 895, 896), user_count=2)` selects1..4 unique
explicit K values. CLI equivalents are `--user-count 2 --key-counts 447 448 895
896`. This exports the existing exit/depth/warmth/zero/full grid for that selection
(up to64 requests); it does not run or count any measurement. Select the eight
leaf/depth2/R0 requests above for the fixed pilot. Invalid K/F, duplicates,
packing failure, lack of a positive full-R endpoint, or existing output paths
fail before new files are written. Default32 grid, IDs and export order stay
unchanged. The independent byte/frame oracle validates F and inherited totals,
reachable contracts, concrete gas/refunds/routes and EIP-150 metadata; its finite
wrapper bound is65,536 instructions. A partial `limit=1` check is not an all-F run.
The oracle defaults to full packing checks; its explicit `enforce_bounds=False`
mode is reserved for out-of-policy R+1 byte-flow tests, never qualification.

F2 keeps all eight R0 controls within30M block gas: cold large-K cases use about
15.1M rather than discarding them because the old F4 campaign packing exceeded
30M. This is not a full30M packing certificate. Sequential raw slots require no
hash search; actual own-input map/mask and late reaccess observations still need
qualification. No model, tariff, guest, SDK or account-journal correction changes.

### Direct-write transient read catalogs

`cases/transient_read_catalogs.py` isolates transient-map reads from the older
store-heavy transient family. One signed CALL to the normal primary (Anchor plus
one user) starts with empty persistent/transient state, guards fixed QUERY=0,
seeds K ordered calldata keys with `TSTORE1`, guards QUERY=1, then executes64
`TLOAD`/`POP` sites per round R. QUERY is last in the catalog. The final guards
check counters, protected memory, retained query and empty stack before STOP.
There are exactly K user TSTOREs and2+64R TLOADs, no user SLOAD/SSTORE, child frame,
cleanup pass, extra prestate code account or transient-slot MPT proof targets.
The K values remain transient until transaction finalization clears them.

The new family reuses the bounded `warm_read_catalogs` key reader/validator and
assembler/base request helpers. K is a strict integer1..1024; keys are unique
canonical U256 hex with exactly eight nonzero bytes each and fixed QUERY last.
The default is mixed/K512. An external JSON file has exactly
`{"schema_version":1,"keys":[...]}` and is limited to128KiB. Changing its key
values preserves code and EVM gas at fixed K/R but does not establish equal input
seed, map layout or actual SP1 workload. Existing family defaults remain intact.

```bash
"$PYTHON_BIN" -S -B bin/zkgas/bench/cases/transient_read_catalogs.py \
  --output-dir transient-read-r64 \
  --model-id 5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11 \
  --probe-repetitions 64
"$PYTHON_BIN" -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p test_transient_read_catalogs.py -v
```

Use the managed Python environment and a fresh output path. Model ID is required;
the CLI does not inherit a legacy model default. `--catalog ordered|mixed`,
`--catalog-file <catalog.json>` and `--key-count K` are optional. Without
`--probe-repetitions` export contains ownzero/full; supplying one intermediate
R exports ownzero/probe/full. The public factories are
`make_case(catalog='mixed', level='full', *, key_count=512)`,
`make_custom_case(keys, level='full', *, key_count=512)` and
`make_probe_case(keys, repetitions, *, key_count=512)`; R64/R512 can be selected
separately without executing full. `prepare(output, *, model_id, ...)` validates
inputs before creating output and rejects existing output paths.

All levels share their K-specific maximal-full signed envelope. Gross gas is
`21484+397K+6775R`; intrinsic is21000+224K and the EIP7623 floor is21000+560K.
Charged gas is max(gross,floor), without refund. At K512, R0 gross224,748 is below
the307,720 floor; R64/R512 charge658,348/3,693,548. The full R is2180 and the
shared signed envelope is14,995,248 (full charged gas plus1000). These are
single-user selections under15M fixture/EIP7825,31M header-prefix,30M charged
block including Anchor and24KiB code bounds, not30M full-block packing claims.
Ownzero still writes all K keys and verifies the successful query.

The independent byte oracle imports no producer gas/assembler. It checks complete
code intent including R0's unreachable body, exact ordered writes, query values,
full transient mapping, stack, protected memory, counters, gas floor, typed
metadata and opcode routes. Its optional body cache proves all64 read sites and
checks live full-state preconditions; direct/cache parity and state/byte mutants
are tested. Real native/SP1 execution performs every instruction. `run_users(row)`
returns explicit `gross_gas`/`charged_gas`, `counts`, `tload_count`/`tstore_count`,
`seed_keys`, `query_values`, `transient_state_at_stop`, `transient_state_after_tx`,
`peak_memory_bytes` and counter/stack details. No ambiguous `gas` alias is used.

Exports are development probes, not IID/holdout samples or fitted pricing data.
Before actual measurement, independently bind canonical GuestInput, signature,
nonce/envelope, receipts, complete world/account/code/witness and the real current
quote. Compare literal actual proposal PGU, node-known and full-proposal estimates
separately. Physical map/hash observations need their own runtime qualification;
semantic key/state checks do not establish capacity, collision or per-read PGU
bounds. Optional incomplete diagnostics remain separate from canonical actual
success. This tool alone makes no tariff, guest/SDK, production, proof or deployment
change; immutable measured data belongs in Atlas.

### Raw transient keys and byte-prefix controls

`transient_read_catalogs.make_raw_case(keys, level='full', *, query_key,
repetitions=None, key_count=512)` is an explicit opt-in factory for arbitrary raw
U256 keys. `query_key` is a strict integer in0..2^256-1, including zero; keys are
bounded unique canonical lowercase32-byte hex and the supplied query must be
last. `level` is zero/probe/full, with a positive intermediate `repetitions`
required only for probe. It returns the existing row/ProgramRequest shape and
marks only its new row parameters with `raw_keys=true`. No new schema, CLI,
runner, interpreter or pricing path is introduced.

The existing `make_case`, `make_custom_case`, `make_probe_case` and `prepare` CLI
retain their fixed query/eight-nonzero-byte policy. Their complete row/request
bytes remain unchanged; source and helper provenance fingerprints naturally
change when implementation source changes. Both modes use the same guarded
kernel and literal VM. In raw mode the query is explicit in code, calldata,
full-state body proof and metadata. The VM still validates the complete transient
mapping, memory and stack on every cached iteration, independently from producer
gas/assembler code.

`zero_hash_catalog(shared_seed, byte_offset, *, key_count=512)` creates a finite
no-search recipe: `query=shared_seed<<64`, candidates
`query | (i<<(8*byte_offset))` for i1..K-1, then query last. Shared seed is a
strict64-bit integer; byte_offset is0..4. Under the observed guest specialization
`H=F(k2^F(k0^S,k1^G),k3^G)`, this recipe gives H0 **only when the guest's actual
shared G equals the supplied value**. It does not qualify a new input, prove its
physical map placements or assert that all candidates are visited. Each actual
input still needs its own seed/mask/hash/owner/value evidence. Full hash0 and
tag0 are valid values and must not be treated as absent diagnostics.

```python
from transient_read_catalogs import make_raw_case, zero_hash_catalog

def prefix_probe(shared_seed, byte_offset):
    keys = zero_hash_catalog(shared_seed, byte_offset)
    return make_raw_case(keys, 'probe', query_key=shared_seed << 64,
                         repetitions=512)
```

For K512 the five offsets have largest low32 groups1/1/1/2/512 and comparison
envelopes C65/65/65/66/512. These are key-shape facts, not measured PGU or
hash-table scan counts. Raw gas uses actual calldata bytes: with n nonzero bytes
and D=32K, intrinsic is21000+4(D-n)+16n, EIP7623floor is21000+10(D-n+4n),
and gross is21484+301K+12n+6775R. Charged gas is max(gross,floor), with no refund.
The shared maximal envelope and full+1 checks are recomputed for that exact
calldata. Lower-level gas helpers require actual calldata when the query differs
from the strict default, preventing silent reuse of the old byte-cost assumption. Code remains fixed-width and identical across offsets at the same
shared seed/K/R.

For a shared seed with eight nonzero bytes, K512 yields4862 nonzero calldata
bytes, intrinsic144880, floor330700 and gross233940+6775R. Ownzero charges330700;
R512 charges3702740 (block3870095 including Anchor). Full is now R2179 with
signed envelope14997665; the old strict-mode R2180/envelope14995248 must not be
reused. Other shared seeds, including zero, have different byte costs and their
own computed full envelope. All unchanged15M transaction, EIP7825, header-prefix,
30M charged-block and24KiB code constraints still apply. This single-user recipe
is not a full30M workload certificate or a measured pricing validation.

### Transient deletion before repeated reads

`transient_read_catalogs.make_raw_case(..., clear_before_reads=True)` opts into
one direct deletion phase in the existing kernel. It requires a strict boolean,
an explicit raw query, and at least two unique query-last keys. The default
`False` preserves the existing rows, requests, IDs and execution behavior. This
option is intentionally not added to the strict factories or export CLI.

The program guards an initially absent query, seeds all keys to one, and guards
the seeded query. It then stores zero to calldata indices `1..N-1`, retaining
index zero as a live key in the same owner's transient map. Native EVM guards
verify the retained value is one and the deleted query is zero before the
existing `64R` TLOAD/POP loop. The deletion counter at memory offset 32 is checked
at termination; memory remains 128 bytes. No persistent slots or extra code are
introduced. Deletion history does not imply a specific physical tombstone layout.

The literal VM checks every seed and deletion in order, keeps the complete live
state, and verifies both direct and cached read-body transitions. In this mode,
`seed_keys` contains all initial keys, `clear_keys` contains `ordered_keys[1:]`,
`query_values` is `[0, 1, 1, 0]`, and `transient_state_at_stop` contains only the
first key with value one. `transient_state_after_tx` is empty. There are `2N-1`
TSTOREs and `4+64R` TLOADs. For the frozen transient ledger this implies
`A=2N+3+64R`, `J=2N-1`, and `U=C*(5N+2+64R)`; `N/M/C` retain the original
materialized footprint after deletion. This is formula bookkeeping, not an
actual-PGU or physical probe-count bound.

Gas, calldata floor, maximal full reference and signed envelope are derived
from the emitted instructions and actual calldata. An R0 control still performs
seeding, deletion and guards. Changed code or input must qualify its own guest
seed/mask and missing-return path before a pressure claim; a previous input's
seed remains only a hypothesis. The existing success-only observer reader is
not a validator for this missing-query phase. Use the existing native/quote/SP1
tools and separately reviewed diagnostics; no new runner is required.

### Compiler-free ECMUL scalar mixtures

`cases/ecmul_scalar_mix.py` builds ordinary `ProgramRequest` data without a
Solidity compiler or a new runner. `make_case(recipe, level, repetitions=None)`
provides three 31-entry recipes: `low` is all scalar zero, `high` is all
`(3 << 252) - 1`, and `mixed` is twelve zero entries followed by nineteen high
entries. `make_custom_case(scalars, level, repetitions=None)` accepts a list or
tuple of 1–64 strict U256 integers. Levels are `zero`, `probe`, and `full`; only
`probe` accepts an explicit positive repetition count below the full reference.
Scalar zero and multiples of the curve order produce the valid infinity output
`(0, 0)`. Nonzero scalars use the existing pure curve helpers; no existing helper
contract or named scalar set changes.

Each 96-byte calldata entry contains the raw scalar and its expected X/Y words.
The program repeatedly STATICCALLs precompile 7 for fixed point `(1, 2)`, with
requested gas 6000 and freshly cleared, disjoint output memory. Every call
checks success, exact 64-byte returndata and both coordinates, including zero
coordinates. Memory offsets 0/32 hold the point, 64 the scalar, 96/128 the
output, 160 a protected zero, 192 the repetition counter, and 224 a protected
sentinel. Peak memory is 256 bytes. The STOP path guards the point, counter and
protected words; no persistent or transient storage is used. Code bytes depend
only on sequence length and repetitions, so scalar mixtures have identical
runtime bytes at the same size and repetition count.

From the repository root, this example validates the six initial reference rows
in memory without exporting inputs or running native/SP1 estimation:

```bash
PYTHONPATH=bin/zkgas/bench/cases:bin/zkgas/bench/tests \
  "$PYTHON_BIN" -S -B - <<'PY'
from ecmul_scalar_mix import make_case
from ecmul_scalar_mix_vm import run_users
for recipe in ('low', 'high', 'mixed'):
    for level in ('zero', 'probe'):
        row = make_case(recipe, level, repetitions=1 if level == 'probe' else None)
        result = run_users(row)[0]
        print(recipe, level, result['gross_gas'], result['charged_gas'])
PY
```

The factory exposes `gas_accounting`, `fits`, `full_repetitions`, `source_modules`
and `helpers` for composition with the existing constructor and quote tools.
Gas includes actual calldata intrinsic/floor, executed opcodes, memory growth,
100-gas warm access and 6000 native gas per call. The automatically solved full
reference respects the existing 15M transaction fixture cap, EIP-7825 cap,
31M header, 30M charged block budget and 24KiB code limit. A sequence uses the
same full-derived signed envelope at every level, with a 1000-gas reserve; the
literal VM additionally checks actual EIP-150 forwarding at each call. `full`
is a generated reference, not evidence of a full-limit SP1 measurement.

The independent byte oracle in `tests/ecmul_scalar_mix_vm.py` decodes every
instruction and interprets every call and iteration. It independently validates
curve outputs, scalar reduction, calldata, memory, stack, counters, gas,
returndata, route counts and typed metadata. It does not import the producer's
assembler, counts or gas functions. Run its regression tests with:

```bash
"$PYTHON_BIN" -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p test_ecmul_scalar_mix.py
```

`source_derived_work_units` records reduced-scalar bit length plus popcount.
These are expected selectors derived from inputs, not exported native A/I/S
accumulators. This family first tests whether a legal mixed input reaches the
intended aggregate-pricing branches. A mathematical aggregate-max difference
alone is not measured underpricing. Literal node-known estimates, full proposal
estimates and actual SP1 PGU must remain separate; no model refit, global bound,
production SDK parity or normal-transaction admission conclusion follows from
generation or byte validation.

### Explicit raw access-list storage keys

`access_list_values.make_raw_case(keys, block_count=1)` opts into one PROGRAM
address with a caller-supplied ordered storage-key vector. Keys must be a list or
tuple of 1–8192 strict U256 integers; zero, maximal values and duplicates are
valid. The request contains `access_list.pattern="raw_keys"`, its exact `count`,
and fixed-width 32-byte hex `storage_keys`. The existing six patterns, 23 default
rows, requests, CLI selection and five-byte empty RETURN runtime remain unchanged.
Old descriptors omit `storage_keys`; non-raw patterns reject a supplied vector,
including an empty vector. Raw descriptors require the vector length to equal
`count`. Block count remains bounded to 1–32.

The new row records raw/unique counts separately and hashes the concatenation of
32-byte big-endian keys in original order. Its sample/corpus identities include
that digest. No sorting, deduplication, storage preload, dummy proof or signed
GuestInput rewriting occurs. The native constructor still validates the canonical
included type-2/type-4 envelope, signer, nonce and complete access-list equality.
The same 15M transaction envelope and ordinary intrinsic-gas enforcement apply;
a syntactically accepted 8192-key descriptor need not fit into a legal transaction.

For example, construct and inspect an in-memory request from the repository root:

```bash
PYTHONPATH=bin/zkgas/bench/cases "$PYTHON_BIN" -S -B - <<'PY'
from access_list_values import make_raw_case
row = make_raw_case([0, (1 << 256) - 1, 0])
assert row['parameters']['expected_access_list_counts_per_block'] == {
    'raw_address_items': 1, 'raw_storage_keys': 3,
    'unique_addresses': 1, 'unique_storage_pairs': 2,
}
print(row['sample_id'])
PY
```

Use a fixture-enabled host constructor containing this opt-in descriptor; an
older pattern-only executable cannot consume it. This host-only feature requires
no GuestInput version, pricing model, guest or SDK change. Generation alone does
not establish guest seed/hash distribution, comparison counts, actual SP1 work
or any pricing deficit. Existing constructor/quote/estimation tools own execution.
Run the Python raw-key and default compatibility tests with:

```bash
"$PYTHON_BIN" -S -B -m unittest discover -s bin/zkgas/bench/tests \
  -p 'test_access_list*.py'
```

### Frozen generic U256 warm-set hash catalogs

`cases/access_list_hash.py` retains the generic U256 hash used by the frozen b76
ELF's access-list warm set. It includes array-length hashing, rotation, both
byte-hash lanes and finalization; the journal fixed32 hash is a different path.
`ELF_SHA256` identifies this scope. `hash_key(key, seed, shared0, shared1)` takes
strict U256/U64 integers. Its expected unit hashes come from exact RV64 words;
static agreement does not establish an actual input's seeds or placement.

`concentrated_keys(shared0, count)` generates distinct keys with numeric little-
endian limbs L0=L1=shared0, L2=0 and L3=index<<32. They share the first28 compared
bytes. If a new input's observed shared0 equals the candidate value, their hashes
are `fold(4, shared0)`, generally nonzero and independent of own seed/shared1.
`dispersed_keys(count)` supplies a deterministic SHA-256 control hypothesis.
Both counts are bounded to1..8192. A zero shared0 is valid and collapses even the
control hashes; no generated catalog is a distribution certificate.

Compose either catalog with the existing raw-list producer:

```bash
PYTHONPATH=bin/zkgas/bench/cases "$PYTHON_BIN" -S -B - <<'PY'
from access_list_hash import concentrated_keys, dispersed_keys
from access_list_values import make_raw_case
candidate_shared0 = 0x243f6a8885a308d3  # Example value, not a measured seed.
candidate = make_raw_case(concentrated_keys(candidate_shared0, 64))
control = make_raw_case(dispersed_keys(64))
assert candidate['parameters']['expected_access_list_counts_per_block'] == {
    'raw_address_items': 1, 'raw_storage_keys': 64,
    'unique_addresses': 1, 'unique_storage_pairs': 64,
}
assert candidate['sample_id'] != control['sample_id']
PY
```

Take a candidate value from a qualified observation, construct a **new** input,
and verify that new input's own map/key/caller/seed/final-hash observations match.
If they do not, retain the valid unrealized case; do not overwrite or retry it as
a successful pressure test. The minimal local G0 watch does not provide G1, so
arbitrary-key full-formula validation must not be claimed when G1 is unknown.
Computed catalog hashes, directly sampled hashes, dynamic placements, comparison
counts and actual complete-proposal PGU are separate evidence. The normal bench
constructor, quote and estimator interfaces continue to own execution.

## Frozen observation plans in the workload harness

To collect optional historical-register diagnostics through `workload.py`, add an
`observation_plan` artifact alongside the existing frozen tools and ELF:

```json
{
  "observation_plan": {
    "path": "programs/observation-plan.json",
    "sha256": "REPLACE_WITH_SHA256"
  }
}
```

The plan must be a schema1 JSON object bound to the configured ELF, at most64KiB.
The harness binds its hash in the existing contract, retains its exact bytes as
`observation-plan.json` in each attempt, rechecks that identity, and passes the
local copy to the sampler with `--observation-plan`. The sampler remains the
authority for executable PCs and all detailed watch/register/sample limits; see
the [observation plan contract](../sp1-estimator/README.md).

Omitting this artifact preserves the existing command and measurement behavior.
Canonical PGU/public-output validation does not depend on optional trace status,
truncation, or a seed/placement hypothesis. Qualify those separately from the
retained raw SP1 report with a pure postreader. No trace qualifier is run by the
harness, and diagnostic replay wall time is not prover time.

## Short-prefix access-list construction candidates

`access_list_hash.concentrated_short_prefix_keys(shared0, count)` emits raw U256
keys with `L0=L1=i` and `L2=L3=shared0`, for `i=1..count`. If that value equals the
new input's **own** G0, both inner folds are zero and the two body terms cancel:
`B=0`, `H=fold(4,G0)`, independently of S/G1. The first four compared little-endian
bytes are distinct, so model389's numeric-low32 groups are singletons and its
construction-pair reserve is zero. Verify that counter in the fresh quote.

The existing long-prefix and dispersed functions are unchanged. The helper's
1..8192 cap is a catalog bound, not an admission promise: the default15M envelope
also limits raw access-list intrinsic gas. Generated equal hashes are conditional
algebra, not observed placements, whole comparison counts or a workload result.
Qualify a new input's own G0/hash/caller context; do not chase a seed or relabel a
valid unrealized sample. A long-family-only postreader needs an explicit short-
family qualifier; its old long-family pressure flag is not evidence about this form.

Generation is ordinary data preparation. From the repository root, start with the
standard frozen artifact configuration at `experiment/tools-config.json` (including
an ELF-matching `observation_plan` if desired). This example seed is only a hypothesis:

```sh
~/.venv/bin/python -S -B - <<'PY'
import copy
import json
from pathlib import Path
import sys
sys.path.insert(0, 'bin/zkgas/bench/cases')
from access_list_hash import concentrated_short_prefix_keys
from access_list_values import make_raw_case
from spawn_memory import digest, encoded, write
out = Path('experiment')
template = json.loads((out / 'tools-config.json').read_text())
row = make_raw_case(concentrated_short_prefix_keys(0x0123456789abcdef, 64))
request = encoded(row.pop('request'))
row.update(request_path='short-k64-request.json', request_sha256=digest(request))
write(out / row['request_path'], request)
write(out / 'short-k64-catalog.json', encoded({'schema_version': 1, 'cases': [row]}))
for phase, limits in (
    ('screen', dict(pss_mib=2048, min_free_mib=576, output_mib=16,
                    log_mib=2, timeout_seconds=180)),
    ('actual', dict(pss_mib=6144, min_free_mib=576, output_mib=192,
                    log_mib=16, timeout_seconds=600))):
    config = copy.deepcopy(template)
    config.update(corpus_id=row['corpus_id'], limits=limits)
    write(out / f'short-k64-{phase}-config.json', encoded(config))
print(row['sample_id'])
PY
~/.venv/bin/python -S -B bin/zkgas/bench/workload.py screen \
  --config experiment/short-k64-screen-config.json \
  --catalog experiment/short-k64-catalog.json --output-root experiment/short-k64-screen
```

The existing harness owns execution. Independently validate the saved native
world/signature/roots, literal quote and planned raw keys before any actual. The
screen retains `input.bin.gz`; the existing `artifacts.restore` API can restore it
to a fresh file using its retained receipt. Then `workload.py measure` accepts that
bincode plus the unchanged `native.json` and `quote.jsonl` through `--native-report`
and `--quote-report`, with the original sample ID, matching actual config and a
fresh output root. It executes only SP1. Keep canonical PGU validation separate
from optional trace qualification; do not pass `--retry-failed` for once-only panels.

### Multiple CALL users sharing an access list

The existing `program.transactions` descriptor can compose with `program.access_list`
for CALL users without `storage_state`. Every user receives the same complete raw
list, with its order and duplicates preserved. Authorizations remain incompatible
with explicit transaction descriptors; CREATE plus access-list, initialized extra
recipients and storage-state combinations remain unsupported. This is an opt-in
host fixture capability; GuestInput and the guest interface are unchanged.

For example, compose a small request using the existing raw-key factory:

```sh
PYTHONPATH=bin/zkgas/bench/cases "$PYTHON_BIN" -S -B - <<'PY'
import json
from pathlib import Path
from access_list_values import make_raw_case

request = make_raw_case([0, (1 << 256) - 1, 0, 1])['request']
request['program']['transactions'] = {
    'kind': 'call', 'count_per_block': 2, 'gas_limit': 15_000_000,
}
request['program']['expected_route_counts']['opcode:0xf3'] = 2
output = Path('experiment/shared-access-list')
output.mkdir(parents=True, exist_ok=False)
(output / 'request.json').write_text(json.dumps(request, indent=2) + '\n')
PY
"$NATIVE_FIXTURE_BIN" --execution-profile evm-gas-only \
  --input experiment/shared-access-list/request.json \
  --output experiment/shared-access-list/input.json \
  --output-bincode experiment/shared-access-list/input.bin \
  --report experiment/shared-access-list/native.json
```

The transaction descriptor retains its existing policy: sender funding1e18,
maxfee1Gwei, zero tip/value, and individually bounded gas limits21,000..15,000,000.
It does not use the old single-user envelope's funding1e16/maxfee25M. Success,
revert and exceptional halt still require every requested signed transaction to
be included in order and every receipt to match `expected_user_success`. Each
included list is checked by canonical EIP2718 decoding, signature recovery,
expected global user nonce and exact list equality. Nonces are
`block_index * count_per_block + user_index`; Anchor remains transaction index0.

The native access-list report still has one `blocks` entry per block. Old
single-user serialization stays unchanged. Multi-user blocks add `transactions`,
with each user's included transaction index, nonce, hash and raw/unique selectors.
Flat block `raw_address_items` and `raw_storage_keys` sum every user, including
raw duplicates. Flat `unique_addresses` and `unique_storage_pairs` are true block
unions: every checked user has the same list, so these equal one user's unique
set, without allocating a second union set. For the example, raw block slots8
and block-unique pairs3 are distinct from summing the nested per-user unique
counts3+3. The legacy top `count_per_block` field remains the descriptor's
cardinality **per user transaction**; multi-user reports also expose
`transactions_per_block` to make that multiplier explicit.

Declared warm slots are not materialized state or witness targets. Real intrinsic
gas, transaction admission and complete block validation remain in force. The
fixture retains its existing31M header limit; an experiment using a stricter30M
workload budget must separately check actual included receipts and each remaining
envelope. No single-user PGU extrapolation establishes a multi-user measurement.
The existing bounded observation reader's single-vector scope may be incomplete
for repeated target-owner items; canonical PGU validation remains independent of
optional profile qualification.

### Explicit raw empty-key access-list owners

`access_list_values.make_raw_addresses_case(addresses, block_count=1)` adds an
ordered list of empty-key owner items. Supply a list or tuple of 1–8192 exact
20-byte `0x`-prefixed hex strings. Zero, maximal addresses and duplicates are
valid descriptor values; hex digits are normalized to lowercase without changing
bytes, order or multiplicity. The descriptor is `pattern="raw_addresses"` with
`count` and `addresses`; it does not insert PROGRAM automatically. All storage
key lists are empty. The raw digest is SHA256 of the ordered 20-byte concatenation;
counts distinguish raw items from unique owners, and storage pair counts are zero.

```bash
PYTHONPATH=bin/zkgas/bench/cases "$PYTHON_BIN" -S -B - <<'PY'
from access_list_values import make_raw_addresses_case
row = make_raw_addresses_case(['0x' + '00' * 20, '0x' + 'ff' * 20, '0x' + '00' * 20])
assert row['parameters']['expected_access_list_counts_per_block'] == {
    'raw_address_items': 3, 'raw_storage_keys': 0,
    'unique_addresses': 2, 'unique_storage_pairs': 0,
}
print(row['sample_id'])
PY
```

`raw_addresses` requires a non-null vector whose length equals `count` and rejects
any `storage_keys` vector. Other patterns reject any `addresses` vector, including
an empty one; `raw_keys` keeps its existing required storage vector. Optional
fields omitted or represented by JSON null retain ordinary `None` semantics and
are omitted when serialized. Old descriptors, 23 default rows/requests and report
shapes are unchanged. Block count remains 1–32.

Declaring an owner does not materialize or preload its account or storage. The
constructor still rejects extra owners already present in its controlled initial
prestate, except PROGRAM itself. Signed type-2/type-4 raw-list equality, signer,
sequential nonce and every included user are checked by the existing gate.
Shared CALL descriptors retain raw counts summed over users and unique counts as
block unions, with per-user counts in nested report rows. The unchanged 15M
transaction envelope, intrinsic gas and inclusion rules still apply: a valid
8192-item descriptor may not fit. No hash recipe, seed selection, dynamic
placement, workload measurement or pricing claim follows from generation.

### Conditional outer warm-owner Address20 catalogs

`access_list_hash.concentrated_owner_addresses(shared0, count, *, prefix_bytes=6)`
returns ordered, distinct, lowercase Address20 hex strings for the frozen b76 outer
`AddressMap` / `FbBuildHasher<20>` path. It uses strict U64 and owner-only count1–16384
validation; `prefix_bytes` must be a strict integer0,4 or6 (no bool). Indexes1..count
fill the remaining bytes of the first8-byte chunk in little-endian order, after a
fixed `A5` plus zero prefix (empty for0). The next8bytes are selected `shared0` LE;
the last4bytes are integer1 LE. Prefix length means the first compared Address
network-memory bytes, not Address numeric low32, generic-U256 limbs or journal keys.

For this exact source profile, `H=F(F(X0 XOR S,X1 XOR G) XOR X2,G)`, where
`F` XORs the low/high64 halves of multiplication. Only if selected `shared0`
equals the **new input's observed own G** do X1=G/X2=1 force H=F(1,G)=G for
any unknown map S and any X0. `G=0` and maximal U64 are valid values. A G from an
earlier input is a hypothesis, and even the same input dimensions do not guarantee
seed reuse. Computed equal hashes do not certify dynamic placements/comparisons,
actual workload or a pricing deficit. The generic-U256 and fixed32 storage-journal formulas differ. Other maps,
including Fb20 account journals, still need their own observed S/G and caller;
matching a hasher type does not transfer this qualification.

For a data-only unit example (unit G17 is not an observed runtime choice):

```bash
PYTHONPATH=bin/zkgas/bench/cases "$PYTHON_BIN" -S -B - <<'PY'
from access_list_hash import concentrated_owner_addresses
from access_list_values import make_raw_addresses_case
addresses = concentrated_owner_addresses(17, 4, prefix_bytes=6)
row = make_raw_addresses_case(addresses)
assert row['request']['program']['access_list']['addresses'] == addresses
assert row['parameters']['expected_access_list_counts_per_block'] == {
    'raw_address_items': 4, 'raw_storage_keys': 0,
    'unique_addresses': 4, 'unique_storage_pairs': 0,
}
print(row['sample_id'])
PY
```

The helper performs no file IO, selection/search, native construction or execution.
Use existing `make_raw_addresses_case` and optional shared-CALL descriptors to
compose requests; unchanged intrinsic gas,15M per-user envelope, controlled-world
prestate exclusions and inclusion rules still apply. Existing canonical native,
Kona/payload/signature/witness checks, quote, complete SP1 workload and input-own
outer profile gates remain necessary. Canonical PGU validity is independent of
optional profile qualification. This adds no runner, mode, price, guest, SDK or
observer change and does not alter old key helpers or the23 default producer cases.

### Raw access-list address-pair reporting

The complete quote exposes `access_list_address_pairs` and
`access_list_address_scaled` in each node/prove block breakdown. The selector is
the sum of `n * (n - 1) / 2` for **individual transaction raw address-item lists**,
including duplicate owners and empty storage-key lists. Node uses included
transaction scopes; prove includes attempted scopes. It is separate from both
storage-key construction selectors and from the existing linear address-item
term. Proposal-once carries zero address pairs and zero address-pair price.

The shared pricing model supplies the tariff and domain. This CLI transports
and validates them; it does not infer guest hash seeds or implement another
pricing formula. Each scaled amount is added once, outside the opcode discount.
Both fields are required by the current complete-quote contract. Regenerate
quotes from retained canonical inputs when evaluating a new model, and join them
to retained actual observations by the input identity. Preserve old quotes and
raw actuals instead of filling missing fields or rewriting their provenance.

### Cross-transaction account-cache requests

`cases/cache_state_accounts.py` generates one ordinary ProgramRequest and catalog,
without execution or a compiler:

```bash
"$PYTHON_BIN" -S -B bin/zkgas/bench/cases/cache_state_accounts.py \
  --output-dir experiment/cache-state-reference
```

The fixed default is one block with512 successful CALL users,512 distinct
predeployed code recipients, empty calldata and no access list. Each recipient
reads eight globally disjoint initially absent Address20 targets using guarded
BALANCE instructions, then returns empty bytes. A nonzero target balance reaches
INVALID. Recipient runtime is215 bytes, with no memory expansion or refund.
The existing explicit-transaction policy retains sequential nonces,1ETH test
funding,1Gwei maxfee,50K user envelopes and31M header. Source arithmetic predicts
41934 charged gas per user and21637563 including the fixed Anchor, below30M;
actual inclusion, receipts and gas remain native qualification gates.

Recipients are `0x4300000000000000000000000000000000000000 + i`, i=1..users.
Reference targets are the first20 SHA256 bytes of
`b'cache-state-absent-control\x00' + i.to_bytes(4, 'big')`, i=1..8*users,
assigned eight consecutive targets to each recipient. Collision, reserved-address
or recipient overlap is rejected; there is no filtering, retry or seed search.
This reference recipe is not an EVM-map hash-distribution certificate.

From the repository root, the Python API also accepts one explicit raw catalog in the same order:

```python
from pathlib import Path
import sys

sys.path.insert(0, 'bin/zkgas/bench/cases')
from cache_state_accounts import make_case, prepare

# Tiny engineering example, not the512-user campaign or an observed seed choice.
reference = make_case(user_count=2)
addresses = list(reversed(reference['parameters']['ordered_targets']))
row = make_case(targets=addresses, user_count=2)
assert row['request']['program']['transactions'] == reference['request']['program']['transactions']
# prepare(Path('experiment/cache-state-raw'), targets=addresses, user_count=2)
```

`targets` must be a list/tuple of exactly eight canonical lowercase20-byte hex
addresses per user, unique across all users and outside recipients, fixed initial
accounts and the reserved low-address range. `user_count`1..512 permits tiny
engineering tests; it does not select a sweep. Each fresh output contains one
request with content-derived IDs, SHA-bound source/helpers and the existing
catalog interface. Use unchanged `workload.py screen` and retained-input
`measure`; no harness wiring or new runner is needed. `--model-id` changes only
catalog metadata, never prices or request behavior.

Changing target immediates changes deployed code/witness/canonical input while
preserving user unsigned envelopes. The deterministic signer and fixed manifest
fields predict unchanged signed user bytes and Kona transaction payload; verify
those relations on retained native outputs instead of forcing a difference.
All target absences,512 code recipients, signatures/order/nonces, exact routes,
roots, success/gas and the actual blob/output limits require independent native
readback before complete-workload measurement.

This probes a block-long `State.cache.accounts`, distinct from the per-TX
journal. Workload address union is not measured physical population. Hash/seed
qualification requires the input's own CacheAccount specialization and caller;
old journal/AL map observations do not transfer. Optional bounded profiles can
remain UNKNOWN/incomplete while canonical PGU remains valid. Cold BALANCE,
Account-density, resource and witness prices may already cover the work. Neither
the recipe nor successful construction establishes pressure, a pricing deficit,
all placements/comparisons, a global bound or SDK production parity. Predeployment
cost is outside this measured block. Prices, Rust, guest, SDK, ELF and old
producer behavior are unchanged.

### Per-transaction access lists and warm-first CacheState cases

Offline `program.transactions.access_lists` optionally supplies one complete
`ProgramAccessList` descriptor per user, in nonce order. This mode requires one
block and ordinary CALL transactions, with no `storage_state`, authorizations or
CREATE. Its length must equal `count_per_block`; it is exclusive with the old
shared `program.access_list`. Each descriptor retains its1..8192 bound. Checked
aggregate raw address items and raw storage keys each have a16384 tool bound,
independently; these are fixture allocation limits, not protocol/admission limits.
Code-only `prestate_contracts` may compose with this mode, including empty initial
storage maps; nonempty initialized storage is rejected. Extra AL owners must still
be absent from the constructor's initial state (apart from its existing PROGRAM
exception), including exclusion of other predeployed recipients.

Signing and the actual committed-user gate select the same nonce-indexed list.
Every included envelope is canonical-decoded, signature/nonce checked and compared
against the exact raw ordered list, preserving duplicates. Raw block counts sum
users; unique addresses and owner/key pairs are true block unions. The new report
has `access_list.mode="per_transaction"`, `transactions_per_block` and per-user
`blocks[].transactions` rows with descriptor `pattern`/`count`. It omits the shared
`pattern` and legacy `count_per_block` fields rather than assigning them a misleading
first-list value. Omitted/shared inputs retain their previous report shape and
semantics. AL declaration establishes warmth, not account materialization or an
absence proof.

`cache_state_warm_accounts.make_case(*, targets=None, user_count=86)` prepares a
finite one-block recipe with128 guarded BALANCE reads per user, disjoint targets,
empty calldata and an exact matching empty-key raw-address list per user. It reuses
the existing guard assembler. The default fixed public G0 control calls
`concentrated_owner_addresses(0, 128*users, prefix_bytes=6)` once, then slices
that ordered vector across users without restarting indices. Each Address20 is
`A5 || zero[5] || indexLE16 || zero[8] || 01 || zero[3]`. This supplies a
structural compression source, not a randomized/dispersed control or observed own
G/placement. Actual one-blob fit remains a native gate: no pretrial/retry, recipe
change or fallback after capacity failure. Generation performs no filtering, seed
search or execution; explicit canonical Address20
`targets` require exactly128*users unique addresses, excluding recipients and
reserved accounts. Smaller1..86 user counts are engineering controls. Each runtime
is3335 bytes. Source accounting is343054 charged gas per user and29669999 for86
users plus Anchor, with400000 user envelopes and a31M header. Native receipts and
full signed/world/absence checks must establish those claims before measurement.
AL values are inside the signed envelopes and Kona payload; changing them can
change signatures/compression and does not preserve the old cold panel payload.

This tiny data-only example neither executes a stage nor creates the86-user case:

```sh
"$PYTHON_BIN" -S -B - <<'PY'
from pathlib import Path
import sys
sys.path.insert(0, str(Path('bin/zkgas/bench/cases')))
from cache_state_warm_accounts import make_case
row = make_case(user_count=1)
assert len(row['request']['program']['transactions']['access_lists']) == 1
assert row['parameters']['expected_block_charged_gas'] == 510409
PY
```

Use the existing `prepare(Path(...), user_count=...)` catalog and standard workload
screen/retained measure interfaces; there is no additional runner. Generation is
not dynamic placement or pricing evidence. The owner-only
`concentrated_owner_addresses` catalog bound is now1..16384; generic U256 helpers
and each raw-address transaction descriptor remain bounded to8192. Generate one
ordered owner vector before slicing into per-user lists; restarting indices per
chunk would duplicate owners. A previous own G is only a prospective hypothesis.
An optional closed CacheAccount profile can supply local own parameters even with
an empty expected-candidate vector; missing/capped evidence and whole placement,
TX identity and population remain UNKNOWN. Canonical PGU validation stays separate
from this optional diagnostic.
