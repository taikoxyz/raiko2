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
