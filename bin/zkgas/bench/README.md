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
and `unique_storage_pairs`. These counts come from the actual included signed transaction
through canonical EIP-2718 decoding, with signer/nonce, exact-list and inclusion gates. No
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
