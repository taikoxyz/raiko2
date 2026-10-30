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
