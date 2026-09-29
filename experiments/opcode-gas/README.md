# Opcode Workload Metric Experiment

This experiment suite is a local, repeatable scaffold for measuring opcode and precompile
contribution to zkVM workload metrics. The direct opcode/precompile lab currently runs on SP1
software `proverGas`; opcode fixtures can run through either the fast synthetic guest or a
revm-backed guest; proposal-level GuestInput runs can also collect RISC0 cycle metrics.

V1 intentionally exports a one-dimensional result shape compatible with the current alethia-reth
Unzen table. Warm/cold storage access, argument-dependent precompile cost, and memory-size sweeps are
tracked as future dimensions, not V1 coefficients.

The multiplier source of truth is the Cargo-pinned
`alethia-reth-evm::zk_gas::unzen::UNZEN_ZK_GAS_SCHEDULE`. Python keeps only experiment metadata
such as names, categories, and fixture templates; it does not duplicate the pricing table. Commands
that need the active schedule obtain it at runtime through:

```bash
cargo run --quiet --locked -p xtask --no-default-features -- export-unzen-zk-gas-schedule
```

`generate` invokes the exporter when a manifest enables schedule-driven expansion, while `damage`
and `inventory` always invoke it. The first such command may pay the one-time Cargo dependency build
cost; subsequent invocations use Cargo's incremental cache. `run`, `run-proposal`, and `fit` do not
load the schedule.

## Commands

Build or refresh the SP1 lab ELFs, then build a release launcher once:

```bash
cargo run -r -p xtask -- build-guest sp1 --bench
cargo build -r -p guest-launcher --features sp1-sdk/profiling
```

### Production Context Campaign Contract

The production-guest context campaign has a frozen manifest for `ADDRESS`, `CALLER`, `CALLVALUE`,
`CALLDATALOAD`, `CALLDATASIZE`, and `TIMESTAMP`. Its implemented CLI covers manifest generation,
preparation, bounded execution/resume, fitting, create-only result sealing, and portable replay.
The canonical inventory is 846 rows. Zero-class promotion is gated by dedicated count-32/count-64
final scenarios for zero-value `CALLVALUE`, empty `CALLDATALOAD`, and nonempty out-of-range
`CALLDATALOAD`. The zero-value holdout uses one byte of explicitly declared non-model calldata so it
has a distinct backend input while remaining in the `value_class=zero` model. The standard versus
gas-estimator parity authority is frozen to `address_canonical`, fit, count 1, target, repeat 0; a
selection or final-holdout row cannot be used for parity.

Generate the canonical manifest into a new path, or validate the tracked manifest and all sealed
source identities it pins:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  generate-context-production-manifest \
  --out experiments/opcode-gas/manifests/sp1-context-production-v1.generated.json
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  validate-context-production-manifest \
  --manifest experiments/opcode-gas/manifests/sp1-context-production-v1.json
```

Generation is create-only and will not replace an existing file. Validation checks the exact V5
operation coverage, sealed higher-layer calibration, and sealed context discovery result, while
the prepare command captures the launcher, production ELF/VK, trace source, clean implementation
revision, parity identity, and every canonical fixture identity in a new run directory. Run and fit
are exact-resume operations over that immutable identity:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  prepare-context-production \
  --manifest experiments/opcode-gas/manifests/sp1-context-production-v1.json \
  --parity-identity /path/to/reviewed-parity-identity.json \
  --out /path/to/new-context-run
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  run-context-production --run /path/to/new-context-run
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  fit-context-production \
  --manifest experiments/opcode-gas/manifests/sp1-context-production-v1.json \
  --run /path/to/new-context-run
```

After the fit terminal exists at the exact clean calibration revision, seal it into a new
content-addressed directory and replay it without SP1 from that same clean, exact source revision:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  seal-context-production-result \
  --manifest experiments/opcode-gas/manifests/sp1-context-production-v1.json \
  --run /path/to/context-run \
  --out-root experiments/opcode-gas/derivations
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  verify-context-production-result \
  --result experiments/opcode-gas/derivations/<result-id>
```

Sealing is create-only and publishes exactly ten bounded regular files through a same-directory
temporary directory. Replay reads numerical inputs only from that flat result directory, but also
requires the current Git HEAD to be the recorded clean implementation revision and recomputes every
frozen source-code digest from bounded, no-follow regular-file reads. Those reads fail closed unless
the Linux kernel can establish and check an exact-path inotify mutation watch for their full
duration. Replay reconstructs every fixture and row identity, recomputes V5 subtotals (including
event dispatch), refits every exact matrix, coefficient, prediction, gate, and model choice, and
preserves accepted, partial, and rejected raw evidence. The discovery result defines only the
feature vocabulary and candidate function shapes: none of its numeric coefficients, the historical
`body_scale`, or a cross-ELF scale is a production-model input.

### Synthetic Four-Anchor Ratio Probe

The dedicated `sp1-opcode-lab` guest has an independent diagnostic mode for the four free relation
anchors `POP`, `PUSH0`, `DUP1`, and `SWAP1`. It is not the REVM transaction guest and does not
produce production opcode costs or a candidate table. It calls REVM's actual `Stack` primitives in
a synthetic loop, so its accepted output is a frozen body-cost prior. The authoritative rank-98
real-REVM relation system and production controlled-block validation remain mandatory. The
production fit consumes the four per-operation slopes only through two shared transfer parameters:
`theta_i = (body_scale * synthetic_body_cost_i + common_opcode_overhead) / raw_gas_i`.

Generate, run, and fit it only inside a freshly frozen calibration run, using the canonical commands
below. The gas estimator is host code, so the evidence binds the calibration revision, SP1 SDK
version, exact guest-launcher binary digest, fixture manifest, raw rows, and opcode-lab ELF. An
artifact copied from an older or standalone host run is diagnostic only and is rejected by the
candidate path.

The frozen fit counts are `[0, 1024, 4096, 16384, 65536]`, the predeclared checkpoint is `131072`,
and every lane has three exact repeats. The probe keeps the historical `OpcodeLabInput` wire schema:
the dedicated opcode-lab binary decodes the equal-length scenarios `anchor_target_` and
`anchor_control` only when the exact case/opcode/envelope declaration also matches. This binary-only
path is deliberate; adding probe state to shared primitives or the shared guest library changes
unrelated proposal, precompile, and REVM guest programs. Serialized bincode length and bytecode
remain fixed across counts; `target_count` is the only count-varying workload value. Both lanes perform the same stack
seeding, loop bookkeeping, one indirect step call, compiler barrier, accumulator mix, and fixed-size
public-output preparation. Lane/opcode selection occurs once before the loop. The control step is a
no-op; the target step calls exactly one REVM `Stack::{pop,push,dup,swap}` primitive.

For each count, define `delta(n) = P_target(n) - P_control(n)` and fit it with a free intercept. This
removes common fixed guest startup from the slope; equivalently, the slope sees
`[P_target(n)-P_target(0)] - [P_control(n)-P_control(0)]`. A nonzero `delta(0)` is allowed fixed lane
overhead, not per-operation cost. The fit requires positive signal, `R2 >= 0.99`, relative slope
standard error at most `0.05`, residual/signal and count-zero intercept residual/signal at most
`0.02`, and marginal checkpoint APE at most `0.10`, where
`predicted = slope * checkpoint_count` and
`observed = delta(checkpoint_count) - fitted_intercept`. The denominator is this positive observed
marginal signal, not total proverGas, so fixed startup cannot hide nonlinear extrapolation. Output fields are
`prover_gas_per_operation` and `prover_gas_per_raw_gas`, with explicit
`synthetic_prior_only=true` and `candidate_eligible=false`.

Target/control subtraction and the free intercept remove fixed ELF startup and fixed lane overhead.
They do not include the production interpreter's per-opcode dispatch/wrapper path because the lab
calls `Stack` primitives directly. `common_opcode_overhead` represents that repeated production
work; it is distinct from startup and from the four block/base costs. A one-scale transfer was
tested and rejected because it could not keep all reconstructed multipliers positive while meeting
the controlled-family error gates.

The old variable-bytecode sweeps are not substitutes. A free intercept removes fixed startup, but
their bytecode, serialized input/deserialization, and helper work still grow with count and enter the
slope. The launcher also requires an opcode-lab guest exit code of zero before single or batch
reports can be published; an obsolete ELF that exits nonzero with empty public values fails closed
instead of becoming a zero-slope row.

First freeze the controlled manifest. `CALIBRATION_RUN` must be the exact directory emitted by
`prepare-calibration`; do not discover the newest run directory. The machine-readable
`--run-path-file` flow captures that directory for every subsequent command:

```bash
RUN_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher \
  --out experiments/opcode-gas \
  --run-path-file "$RUN_PATH_FILE"
CALIBRATION_RUN="$(<"$RUN_PATH_FILE")"
```

The candidate-bearing probe must be generated after that freeze and persisted inside the exact run.
The consumer checks that its ELF hash equals the `sp1_opcode_lab.elf` hash frozen in
`experiment.json`; the `/tmp` diagnostic above is not accepted as a substitute for this canonical
artifact path:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --out "$CALIBRATION_RUN/generated/anchor-probe"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --fixtures "$CALIBRATION_RUN/generated/anchor-probe" \
  --out "$CALIBRATION_RUN/raw/anchor-probe.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-anchor-probe \
  --calibration-run "$CALIBRATION_RUN" \
  --runs "$CALIBRATION_RUN/raw/anchor-probe.jsonl" \
  --out "$CALIBRATION_RUN/anchor-probe-fit.json"
```

### Authoritative Relative-Relation And Block-Calibration Flow

Run these commands in this order. They use the exact `CALIBRATION_RUN` emitted above; do not replace
it with a discovered or manually selected run directory.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-relations \
  --manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --calibration-run "$CALIBRATION_RUN" \
  --out "$CALIBRATION_RUN/generated/formal-relations"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-relations \
  --fixtures "$CALIBRATION_RUN/generated/formal-relations" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/raw/formal-relations.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-relations \
  --runs "$CALIBRATION_RUN/raw/formal-relations.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/opcode-relations.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-block-calibration \
  --guest-launcher target/release/guest-launcher \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --out "$CALIBRATION_RUN/block-calibration-rows.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-dynamic-opcode-models \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --runs "$CALIBRATION_RUN/block-calibration-rows.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/dynamic-opcode-models.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-core-opcode-submodel \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --dynamic-models "$CALIBRATION_RUN/dynamic-opcode-models.json" \
  --out "$CALIBRATION_RUN/core-opcode-submodel.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-block-calibration \
  --relations "$CALIBRATION_RUN/opcode-relations.json" \
  --anchor-probe "$CALIBRATION_RUN/anchor-probe-fit.json" \
  --runs "$CALIBRATION_RUN/block-calibration-rows.jsonl" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out "$CALIBRATION_RUN/block-calibration.json"
```

### Corrected evidence derivation

`derive-core-opcode-submodel` is the only supported way to persist a corrected
schema-4 dynamic artifact and schema-3 core artifact from an immutable historical
run. It is a CPU-only post-processing replay: it never generates fixtures, launches
a guest, opens a proposal, or changes the source calibration directory.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py derive-core-opcode-submodel \
  --source-run experiments/opcode-gas/runs/09ebb08d76d3f461086b0cf4 \
  --out-root experiments/opcode-gas/derivations
```

The command requires a clean analysis checkout and a locally resolvable historical
implementation commit. It replays the source experiment and provenance identity,
frozen manifest, all adaptive relation round hashes, accepted relation artifact, every
validated anchor guest-input byte and its fixture/raw/fit chain, and raw block rows.
`fit-dynamic-opcode-models` semantically validates those block rows and fits the
transfer parameters; the derivation does not replay or claim a legacy block-calibration
artifact, whose positive-multiplier holdouts are incompatible with declared-zero controls.
It rehashes that same exact source-file set at the end of replay. It publishes a new
create-only directory `experiments/opcode-gas/derivations/<derivation-id>/` with
`derivation.json`, `dynamic-opcode-models.json`, and `core-opcode-submodel.json` only
after dynamic support and core exact replay both succeed. An unsupported dynamic result,
static rank failure, or replay failure publishes no derivation directory or partial file.
The envelope binds the old calibration ID and revision separately from the current clean
derivation revision, all source byte and semantic hashes, and the output content/file
hashes. It is always
`candidate_eligible=false`; a derivation is not a new calibration identity and cannot
be used for a candidate or proposal command.

`fit-dynamic-opcode-models` is a non-candidate diagnostic. The first completed calibration,
re-evaluated with anchor body costs divided by anchor raw gas in the opcode-lab unit system, showed
that one scalar lab-body `proverGas / raw EVM gas` multiplier does not describe `EXP`, `KECCAK256`,
`MLOAD`, `MSTORE`, `MSTORE8`, or `MCOPY` across their frozen scenarios. The command therefore
recovers each target opcode's body cost from its signed target/control relation and fits a frozen
semantic model:

```text
f_mem     = beta_event * memory_growth_event
          + beta_evm * memory_evm_gas_delta
          + beta_boundary * memory_4k_boundary_event
EXP_body   = small_bucket_body                                      if exponent_bytes <= 4
           = max(small_bucket_body,
                 beta_0 + beta_b * exponent_bytes
                 + beta_b2 * exponent_bytes^2)                      otherwise
keccak_permutations = 0 if input_length == 0 else floor(input_length / 136) + 1
keccak_zero_length_event = 1 if input_length == 0 else 0
KECCAK256  = beta_0 + beta_z * keccak_zero_length_event
           + beta_p * keccak_permutations + f_mem
MLOAD      = beta_load + f_mem
MSTORE     = beta_store + f_mem
MSTORE8    = beta_store8 + f_mem
MCOPY      = beta_0 + beta_w * copy_words + f_mem

production_cost = body_scale * body_cost + common_opcode_overhead
```

`small_bucket_body` is the maximum measured body cost across the accepted EXP byte-length
`0,1,2,4` rows. Those rows are declared conservative-approximation evidence, not polynomial fit
rows. The quadratic is fit only at byte lengths `8,16,32`, validated at `24`, and floored by the
bucket throughout the greater-than-four integer domain. The bucket may overpredict short exponents;
it may never underpredict a measured short-exponent row.

`build-core-opcode-submodel` independently replays the accepted relation and schema-4 dynamic
evidence, fits nonnegative static opcode bodies, stores common dispatch exactly once, and replaces
the six structured opcodes above with typed models. Its static NNLS basis contains only equations
with neither a structured-dynamic opcode nor `opcode:0x19` (NOT) or `opcode:0x5b` (JUMPDEST).
Those two keys are declared dispatch-only approximations: their opcode-specific bodies are fixed to
zero, while each event still pays `common_dispatch` once. A dynamic target/control relation may use
one of them only as that declared zero-body control; it must never deconvolve the old affine body
estimate. The solver fails closed if the reduced static system loses rank.

Schema-3 permits one explicit partial-coverage outcome: a non-anchor, non-dispatch static key may
be marked `only_dispatch_dependent_evidence` only when it has a zero coefficient in every eligible
ordinary equation. It is then unsupported rather than assigned a coefficient. Any accepted relation
that references that key is retained as source evidence but has the final outcome
`unmeasured_unsupported`, with no invented numeric prediction or residual. All fully modeled
relations, including `MLOAD - NOT`, are replayed numerically from the final typed registry.

Relations containing either dispatch-only key remain source evidence but cannot fit another static
coefficient or enter the ordinary 5% MAPE, 10% maximum-APE, or exact-flat admission gates. When all
referenced opcodes are modeled, schema-3 records the final typed-registry prediction, residual, and
diagnostic error. A relation whose static target has only dispatch-dependent evidence instead records
the observed slope and explicit unsupported outcome without fabricating a prediction or residual.
Schema-3 retains the static-fit diagnostics separately from this final-registry replay.

The core inventory has 103 of the 150 named Unzen opcodes. A schema-3 artifact may explicitly omit
a core key only under the zero-eligible-coefficient policy above; all other named opcodes outside the
core inventory remain explicitly unsupported. This is a partial, `candidate_eligible=false` artifact.
It cannot open or participate in proposal validation; the remaining opcode families and higher
estimator layers must be calibrated and sealed first.

The existing Prague-derived 101-supported core coefficients remain the sealed historical baseline
and are reused unchanged. The Osaka follow-up samples only ISZERO and CLZ; an augmented artifact must
bind both the baseline and this supplemental evidence without claiming a full 103-key resample.

### Stateful SLOAD/SSTORE campaign

Run the stateful campaign only from the repository root and only after the reviewed
`sp1_revm_opcode_lab` ELF/VK/provenance refresh is committed and the worktree is clean. The
campaign executes 1,128 target/control rows (40 predeclared scenarios, their declared counts, and
three repeats) through the production SP1 gas-estimator path. Fixture generation, verification,
exact fitting, and result replay normally take seconds to a few minutes; formal guest execution is
the long step and can take hours depending on the host. It is resumable from its immutable per-row
records and should not be wrapped in an optimistic short timeout. The runner uses a fixed chunk of
eight complete target/control pairs (at most 16 missing inputs; a partial-pair resume may contain
only the missing lane). Each fully validated chunk is persisted immediately, so an interruption
loses at most the current chunk. Terminal rows and decision files are created only after every row
is complete; chunk size never adapts to observed reports.

Prepare the calibration identity, generate the frozen fixtures, execute/resume the run, and replay
the terminal Task 4 evidence with:

```bash
RUN_PATH_FILE=target/zkgas-stateful-calibration.path
STATEFUL_FIXTURES=target/zkgas-stateful-fixtures
STATEFUL_RUN=target/zkgas-stateful-run

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher \
  --out experiments/opcode-gas \
  --run-path-file "$RUN_PATH_FILE"
CALIBRATION_RUN="$(<"$RUN_PATH_FILE")"

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-stateful \
  --manifest experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json \
  --out "$STATEFUL_FIXTURES"

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-stateful-opcode-campaign \
  --manifest experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json \
  --calibration-run "$CALIBRATION_RUN" \
  --fixtures "$STATEFUL_FIXTURES" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --out "$STATEFUL_RUN"

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py verify-stateful-opcode-campaign \
  --manifest experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json \
  --calibration-run "$CALIBRATION_RUN" \
  --fixtures "$STATEFUL_FIXTURES" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --run "$STATEFUL_RUN"
```

Seal only through the verified-run entrypoint. There is intentionally no `--rows` or precomputed
model-report option:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py seal-stateful-opcode-result \
  --manifest experiments/opcode-gas/manifests/sp1-stateful-opcode-v1.json \
  --calibration-run "$CALIBRATION_RUN" \
  --fixtures "$STATEFUL_FIXTURES" \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf \
  --run "$STATEFUL_RUN" \
  --out experiments/opcode-gas/derivations

RESULT_DIR=experiments/opcode-gas/derivations/<result-id-printed-by-seal>
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py verify-stateful-opcode-result \
  --result "$RESULT_DIR" \
  --guest-launcher target/release/guest-launcher
```

The calibration directory, fixtures, and campaign run are generated/ignored evidence. The stateful
manifest, sealed 103-opcode source registry, and this operator documentation are tracked inputs.
After successful replay, the one flat content-addressed result directory under
`experiments/opcode-gas/derivations/` is the only generated result intended to become tracked. Its
verifier reads that directory, checks an exact nine-file inventory, regenerates every canonical
fixture, and uses the hash-bound reviewed launcher only for host-native REVM identity replay; it
performs no SP1 guest execution.
It accepts an evidence-only descendant commit only when every fitting source, manifest, registry,
and guest artifact remains byte-identical to the measured revision.

This campaign measures stateful REVM execution, including storage execution, journal updates, and
result-state construction. It excludes witness materialization, persistent dirty-state commit, trie
hashing, and final-root construction. The sealed result always remains
`candidate_eligible=false`, `proposal_validated=false`, and
`production_registry_modified=false`. Do not run proposal replay, edit the production registry or
multiplier table, change runtime configuration, or promote the result under this workflow.

The formal 2026-09-29 campaign is sealed as
`experiments/opcode-gas/derivations/64065fa462311bdc1848e9d0`. It contains 1,128 rows from 564
target/control pairs and selects `M_typed`; `M_fixed` and `M_access` are rejected. All 18 primary
scenarios pass, and the selected model's maximum holdout, checkpoint, and high-limb APE are about
`0.2714%`, `0.2068%`, and `4.0451%`, respectively. See the calibration progress ledger for the
exact model parameters and identities.

The selected typed model is evidence, not a directly usable proposal estimator. The current
proposal trace cannot distinguish access warmth and SSTORE original/current/new relationships
reliably. A separate reviewed promotion change must emit those execution-time fields and retain
persistent dirty-state/trie work as a higher-layer cost. In particular, do not reinterpret the
measured negative cold modifiers as a generic unsigned cold surcharge and do not infer the typed
branch from final raw gas.

### Bounded Osaka supplement

Run these commands from the repository root. First verify the exact tracked historical-schema
fixture; its checksum record intentionally contains a repository-relative path:

```bash
sha256sum --check experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.sha256
sha256sum --check experiments/opcode-gas/tests/fixtures/historical-core-102/historical-evidence.sha256
```

`historical-evidence.json` is the small, content-addressed historical canary package: it carries
the eleven selected relation observations and canonical target/control program identity evidence.
It makes baseline validation self-contained; no ignored historical `runs/` directory is read.

The bounded runner reads the calibration directory from the durable path file. It executes only the
eleven frozen compatibility relations at their historical selected counts and, after that canary
passes, the adaptive ISZERO and CLZ relations. It uses three repeats and the existing relation
generation, execution, fitting, precision, checkpoint, and decision-replay machinery.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-osaka-opcode-supplement \
  --run-path-file experiments/opcode-gas/runs/current-run.path \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_revm_opcode_lab.elf

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py verify-osaka-opcode-supplement \
  --run-path-file experiments/opcode-gas/runs/current-run.path \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml
```

The run writes fixtures, raw rows, a sealed adaptive decision ledger,
`compatibility-canary.json`, and `opcode-supplement.json` below
`runs/<calibration-id>/osaka-opcode-supplement/`. Resume accepts persisted rounds only after exact
row, result, provenance, and hash replay. Verification never invokes a guest.

Seal and replay the review-only augmentation with:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py seal-osaka-opcode-augmentation \
  --run-path-file experiments/opcode-gas/runs/current-run.path \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  --out-root experiments/opcode-gas/derivations \
  --augmentation-path-file experiments/opcode-gas/derivations/current-osaka-augmentation.path

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py verify-osaka-opcode-augmentation \
  --augmentation-path-file experiments/opcode-gas/derivations/current-osaka-augmentation.path \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml
```

The create-only augmentation directory contains exactly `augmentation.json`, the canary, the
supplement, and the augmented core artifact. Its identity binds the immutable historical baseline,
all source rows and decisions, current manifest and schedule hashes, Osaka guest and analysis
revision, and the complete structured version identity. The verifier reads only that directory and
the declared historical fixture, then recomputes every content/file hash and both supplemental
equations. This workflow does not sample proposals, rerun the historical 101 coefficients, write a
production multiplier table or config, or alter Boundless behavior. A failed canary requires a
separately reviewed recalibration; it never launches a full resample automatically.

The two supplemental bodies are sealed both as exact rational numerator/denominator pairs and as
80-digit, half-even Decimal registry projections. The verifier replays the rational equations
exactly and bounds the Decimal projection residual. If pointer publication fails after the
content-addressed directory is sealed, rerun the identical seal command: it validates and reuses
only that exact directory to create the missing pointer. An existing pointer remains create-only
and is rejected before any directory publication.

The bounded Osaka supplement completed on 2026-09-26 from implementation revision
`571dd487ffef8c3e158a376e13c9e159da7c936c`. Calibration
`51f71fde68f378842f872fc1` used REVM opcode-lab ELF SHA256
`a4d340812a54a36ce57cdd0f197843f43f67fd9ac7450acee2ec1f6e58eb9a56`. The eleven-relation
compatibility canary passed with drift MAPE `0.00031986761894294409` and maximum per-relation APE
`0.00082987551867219917`. ISZERO and CLZ were both accepted at generator bound 32. The create-only
augmentation is `f945e67bb2c38c9c8ef50530`; its core artifact models 103 of 150 named opcodes,
leaves 47 explicitly unsupported, reuses all 101 historical models, and has artifact SHA256
`b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b`. It remains
`candidate_eligible=false`.

The completed operator sequence used the durable path files under `target/`:

```bash
cargo build -r -p guest-launcher --features sp1-sdk/profiling
sha256sum --check experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.sha256
sha256sum --check experiments/opcode-gas/tests/fixtures/historical-core-102/historical-evidence.sha256
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher --out experiments/opcode-gas \
  --run-path-file target/zkgas-osaka-run-path
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-osaka-opcode-supplement \
  --run-path-file target/zkgas-osaka-run-path \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  --guest-launcher target/release/guest-launcher --elf crates/guests/elf/sp1_revm_opcode_lab.elf
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py verify-osaka-opcode-supplement \
  --run-path-file target/zkgas-osaka-run-path \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py seal-osaka-opcode-augmentation \
  --run-path-file target/zkgas-osaka-run-path \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  --out-root experiments/opcode-gas/derivations \
  --augmentation-path-file target/zkgas-osaka-augmentation-path
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py verify-osaka-opcode-augmentation \
  --augmentation-path-file target/zkgas-osaka-augmentation-path \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml
```

No proposal input was opened and no production table, runtime configuration, or Boundless behavior
changed during this run.

The run-level supplement verifier is a frozen-revision check: it requires `HEAD` to equal the
calibration implementation revision and permits only generated experiment paths to be dirty. Run it
before updating result documentation. After the result commit changes `HEAD`, use the self-contained
augmentation verifier for portable replay; it does not depend on the ignored runtime directory.

Block-level validation must report the frequency-dependent residual from NOT, JUMPDEST, and the
small-EXP bucket. A later fixed offset may not absorb or hide those approximation errors.

Historical pre-P1 sealing record (superseded model representation): run
`09ebb08d76d3f461086b0cf4`, frozen at implementation revision
`09cced01`, accepted all 170 terminal relations. Its canonical accepted stream has 8,124 raw
execution rows; the five adaptive rounds executed 6,120, 4,608, 2,400, 1,368, and 1,008 rows at
bounds 8, 32, 128, 512, and 2048 respectively. The ordinary static relations were accepted with
MAPE `1.519e-60` and maximum APE `2.433e-59`. Four declared approximation relations have maximum
absolute residual `17.3626393645827`; NOT (`opcode:0x19`) and JUMPDEST (`opcode:0x5b`) remain the
two dispatch-only keys.

The historical pre-P1 schema-4 dynamic artifact was `supported` (14 parameters/exact rank 14): EXP fit MAPE/max APE
are effectively zero and its holdout maximum APE is `0.006415%`; KECCAK256 is
`0.534789%`/`1.378295%`/`0.675443%` (fit MAPE/fit max/holdout max), and MCOPY is
`3.235224%`/`8.117928%`/`0.616077%`. EXP's conservative small bucket is body
`2375.1768915483163` and production `2703.2768343220275`; across byte lengths 0, 1, 2, and 4 it
has maximum overprediction `1963.6428571428571` and zero underprediction. The artifact digest is
`fb750c6427b8ebb57264be19500edac8617f06356a42e3881d7eea2399e43689`.

The superseded pre-P1 schema-2 core artifact was `supported_core_submodel`, replayed exactly, and has artifact digest
`e0a358ece713056e075c2d5b131d6c9c029db6e3fdd982bbfd075e65f6accf10` (file SHA256
`833617e238ff2ba63f0cd25b8af312900c288d805d078931c5a3212f5551fe05`). It models 102 of
150 named opcodes and leaves 48 explicitly unsupported, so it is
`candidate_eligible=false`. No proposal row was opened and no production table or configuration
changed.

P1 basis correction: the preceding schema-2 core artifact is preserved historical evidence, not a
current core model. It allowed a dispatch-containing equation to contribute to static NNLS and kept
pre-replacement static predictions after typed dynamic models were installed. The sealed raw stream
was refit under the declared-zero dynamic-control basis through derivation
`3e1d97c461cd2ef9a40e6a02`, produced by analysis revision `a99f8933`. The schema-4 dynamic result
remains `supported` with exact aggregate rank 14/14 and digest
`6b2225662e4bd678d8a9e94983b5ebeccf173a92d358d5072604419c962dcfa8`.

The reduced static system has 90 non-anchor, non-dispatch columns and exact rank 89;
`opcode:0x15` is the sole zero column. Its only rank-restoring relation,
`opcode:0x15:canonical`, uses NOT and is therefore explicitly marked
`only_dispatch_dependent_evidence`. The resulting sealed schema-3 core build is
`supported_core_submodel`, models 101/150 named opcodes, and has digest
`900964c9e64af23c35e71d05f118c519d7b9dfee5263d201af7f850b9de8c964`.
`opcode:0x15:canonical` is preserved as `unmeasured_unsupported`; the actual MLOAD-NOT final
typed-registry prediction is `223.8117854914965440220973490660719690350`, versus observed
`223.9341397849462365591397849462365591398` (absolute residual
`0.1223542934496925370424358801645901048`). The derivation envelope digest is
`b61fda990d258ea8dbb909572d0df8efb12adca0b14e8bb01bc8f2c437a33115`; it binds 70 directly
hashed source files, including all 48 anchor GuestInputs. The historical run was not overwritten,
and no guest execution, proposal row, production table, or configuration change was made for this
correction.

Fresh sealing run `ff00067694b841894e53c921`, frozen at implementation revision `26f899ea`,
accepted all 170 formal relations after the frozen adaptive rounds. The independent nonnegative
static-body replay under the old undeclared-approximation policy was `not_supported`:
`opcode:0x19` and `opcode:0x5b` were active-zero keys,
nonzero-relation MAPE was 1.2364%, and maximum APE was 56.6756%, failing the frozen maximum-error
gate. The schema-3 dynamic fit was also `not_supported`. Its EXP zero-byte fit row had actual and
predicted production costs 479.1089 and 155.5262 proverGas respectively, for 67.5385% APE; aggregate
EXP fit MAPE, fit maximum APE, and holdout maximum APE were 20.4370%, 67.5385%, and 18.1747%.
KECCAK256, MCOPY, and the shared-memory model remained supported, but aggregate support correctly
failed closed. The dynamic evidence digest is
`0bcce47c65dde637c7411ab5b815390fa8f73cca125069e823527c65f1fdd874`.

The then-current schema-1 core-submodel command consequently failed closed with `schema-3 dynamic
opcode artifact header/schema is invalid`; no `core-opcode-submodel.json` or core artifact SHA256
was emitted. This run is preserved unchanged as the single frozen `not_supported` result under the
old policy. It is not relabeled by the schema-4/schema-2 addendum. Its thresholds, features,
fixtures, and source revision were not tuned after observing the failure, and no replacement run
was started.

Run `25db29d91bd3311441fe8317` showed that the former linear 4-KiB page-count term fit its training
rows (production MAPE 0.259%, maximum APE 1.103%) and passed the `0x0800` and `0x0fe0` holdouts, but
overpredicted `0x2000` by about 42% for all three memory opcodes. Replacing the count with the binary
`memory_4k_boundary_event` lowered those viewed-row errors to 5.72%, 5.79%, and 5.96%. Because that
row informed the revised hypothesis, `0x2000` is now fit/diagnostic evidence and cannot remain a
holdout.

The then-current 66-scenario matrix had aggregate exact rank and parameter count 13; its shared-memory
fit has exact rank six. For each memory opcode, the reused warmed rows at offsets `0x0100` and
`0x1000` remain zero-growth holdouts, `0x0800` and `0x0fe0` are retained after passing the prior
model, and `0x4000` is the fresh untouched expansion holdout. Production-space fit MAPE must be at
most 5%, and
fit/holdout maximum APE must be at most 10%. A quality failure is preserved as `not_supported`; it
does not abort the schema-3 diagnostic artifact, modify a coefficient after seeing results, or enter
candidate construction. The revised manifest requires another fresh calibration identity; the run
above motivated the change but cannot validate it.

Fresh run `8e2abe743f28f69ec2f89c8b` validated the revised shared-memory hypothesis. The shared fit had
exact rank six, 1.926% production MAPE, 7.416% fit maximum APE, and 8.887% holdout maximum APE. The
previously untouched `0x4000` production-space holdouts were 7.223% for MLOAD, 7.288% for MSTORE,
and 7.410% for MSTORE8, all below the frozen 10% gate. The aggregate artifact is nevertheless
`not_supported` and `candidate_eligible = false` because the independent KECCAK256 model failed its
fit gates (10.318% MAPE and 29.384% maximum APE). EXP, MCOPY, and the shared-memory family are
supported; the aggregate status must not be interpreted as a shared-memory failure. That run used
the superseded `input_words` KECCAK256 model, so its KECCAK256 failure is motivation rather than
validation evidence for the permutation model.

KECCAK256 now has 22 fit rows, including its canonical row and all previously viewed evidence, plus
seven frozen untouched holdouts at 95, 333, 543, 544 (with 17 warmed memory words), 545, 1500, and
4096 bytes. The fit rows exercise both sides of the 136- and 272-byte padding boundaries; the new
holdouts add the 544-byte padding boundary and distant extrapolation. A fresh manifest identity and
calibration run must validate this matrix; the shared-memory evidence from run
`8e2abe743f28f69ec2f89c8b` remains valid prior evidence because that family and its rows are
unchanged.

Run `77cff7accc62325aad8c047b` then accepted all 162 formal relations and confirmed the nonzero
permutation model on all seven then-frozen holdouts (1.841% maximum production APE), but the two-parameter
fit failed because the zero-length fit row had 232.621% APE. REVM returns `KECCAK_EMPTY` for that
row without entering the nonempty hash/syscall setup path, so one shared constant was the wrong
semantic model. A read-only diagnostic with the explicit zero-length event reduced fit MAPE to
0.603%, fit maximum APE to 1.388%, and holdout maximum APE to 0.801%. Those viewed results motivate
the three-parameter model but do not validate it, and those seven viewed holdouts are now fit
evidence. The implementation revision and replacement holdouts require a fresh calibration identity
and run. The zero-length coefficient has no distinct length-domain holdout; support for that branch
is limited to the REVM fast-path semantics plus its three repeated fit measurements, while the fresh
holdouts validate only the nonzero permutation relation.

Fresh run `e8663a09b3f35da37196cbdf`, frozen at implementation revision `3b9ba3b3`,
accepted all 169 formal relations and produced a full-rank 14-parameter structured model with
`status = supported`. KECCAK256 used 22 fit rows and seven untouched holdouts: production-space fit
MAPE was 0.535%, fit maximum APE was 1.378%, and holdout maximum APE was 0.675%. The holdout APEs
for 95, 333, 543, warmed 544, 545, 1500, and 4096 bytes were respectively 0.488%, 0.029%, 0.675%,
0.308%, 0.291%, 0.052%, and 0.007%. Its fitted production-space term was approximately
`678.14 - 500.51 * zero_length_event + 2147.09 * keccak_permutations + f_mem`. EXP, MCOPY, and the
shared-memory family also remained supported; the shared-memory holdout maximum APE remained 8.887%.
The diagnostic is deliberately `candidate_eligible = false`. The old scalar block-candidate path
still fails closed on a nonpositive reconstructed lab-body scalar for static `opcode:0x19`, before
reaching dynamic-scalar validation. This is a separate static affine/scalar issue; it neither
validates nor invalidates the structured dynamic result.

A replay audit of this run found that direct library calls to `fit_formal_relation_round` inherited
the caller's Decimal precision, while the CLI already ran at the required 80-digit precision. The
round fitter now isolates that context directly, and default-precision replay reproduces the sealed
169-relation ledger byte-for-byte. This fix does not change the sampled rows or fitted artifact, so
the run remains evidence for revision `3b9ba3b3` and does not require resampling.

`fit-block-calibration` deliberately retains the scalar lab-body dynamic holdout gate and therefore
remains fail-closed until a separately reviewed promotion defines the production representation.
A lab-body scalar pass would not by itself prove that one production per-raw-gas scalar is valid:
the nonzero common per-operation overhead must also be represented explicitly when raw gas varies.

`initial_memory_words` is executable fixture state, not descriptive metadata. Dynamic memory
relations allocate exactly that many words with the fixed warmup before the measured opcode. This
lets equal-size operations vary memory growth independently; changing only the manifest field would
produce a falsely full-rank design matrix over identical executions.

`run-relations` owns the frozen per-relation adaptive bounds `8, 32, 128, 512, 2048`. The initial
bound-8 batch must contain the complete manifest relation set and pass the full frozen dynamic
preflight. Later rounds regenerate and execute only relations whose prior failure was limited to the
frozen fit, signal, R2, residual, or checkpoint gates. Trace, repeat, provenance, raw-gas, schema,
fixture, or identity failures abort instead of expanding. Each round is sealed in
`formal-relation-decisions.json`; resume replays every recorded raw/result hash and skips relations
already accepted. The canonical `raw/formal-relations.jsonl` is created atomically only after every
relation has accepted exactly one generator bound.

The sealed decisions ledger, not the final JSONL by itself, is the downstream source of truth.
Relation fitting, block-calibration execution/fitting, candidate construction, and candidate/bridge
replay verify `formal-relation-decisions.sha256`, replay every decision, require a complete accepted
terminal state, reconstruct the canonical accepted rows, and then require
`raw/formal-relations.jsonl` to match those canonical JSON lines byte-for-byte and in order. The
candidate provenance binds `formal_relation_decisions_sha256` in addition to the relation artifact
digest.

Relation `count` is the number of active target-microprogram repetitions in the fixed footprint. It
is not transaction count, block count, gas limit, or a value recovered from runtime gas. Non-self
OLS uses only positive fit counts (`count >= 1`); count zero never enters slope, R2, standard error,
residual, signal, or checkpoint baselining. The artifact reports
`positive_fit_intercept_p`, `zero_delta_p`, and signed
`activation_gap_p = zero_delta_p - positive_fit_intercept_p`. A positive gap means count zero lies
above the positive-count line's extrapolated intercept. `activation_gap_ratio` is
`abs(activation_gap_p) / signal_p` when signal is positive. These fields and the tail evidence are
part of formal relation artifact schema version 3; version-1 and version-2 relation artifacts cannot
enter the new candidate path. Version 3 additionally binds each dynamic row's `model_split`.

Every selected relation round pre-generates and executes an `active_tail` count-one pair. Its target
microprogram executes in the final slot; ordinary count one executes in the first slot. Their
raw-gas multisets and footprint match, while structured placement/sample identity keeps grouping,
raw order, three-repeat completeness, resume replay, and hashes distinct. If activation-gap ratio
is strictly greater than `0.02`, the tail becomes an acceptance gate: its observed marginal is
`delta_tail - delta_zero`, its prediction is `slope_p`, signs must match, and APE must be at most
`0.10`. Both zero passes exactly; one zero or opposite signs fails. With zero positive signal, a
zero gap reports ratio zero and does not trigger, while a nonzero gap reports an explicit
`zero_signal_nonzero_gap` status and must trigger. Untriggered tail results remain diagnostics; JSON
never uses Infinity or NaN.

Count and placement remain host-only sampling metadata. The serialized guest-visible `case` is
stable per lane (`<case>__relation_target` or `<case>__relation_control`) across every count and
placement; consequently all control inputs at one generator bound are byte-identical, while the
prefix-one and tail-one target inputs differ only in bytecode slot order. Replay reconstructs the
entire canonical case and guest declaration from the manifest relation and generation formulas—not
from persisted scenario, relation scenario, gas limit, opcode/count, length, maps, or fixture
metadata. It then recomputes pair, workload, and execution-row identities and joins them to the
controlled trace. Identical canonical inputs may intentionally reuse workload/execution identities;
arbitrary relabeling, declaration changes, or reuse fails exact recomputation.
Every opcode fixture's `evm_opcode_counts` field records the static EVM bytecode multiset. The raw
report field `opcode_counts` remains the SP1 RISC-V execution profile; both are retained under those
distinct names, and any opcode fixture/report key collision is rejected before the raw row is built.

An exactly constant non-self target-minus-control response across positive fit points and checkpoint
is a valid zero-slope relation; count zero then remains activation evidence. Self-controls retain
all-count exact-flat semantics, including the tail placement. A non-self exact-flat row still binds
its signed raw-gas coefficient map and all ordinary trace/provenance evidence; non-flat small signals
keep the ordinary signal gate.

The sealed failed run `runs/1bee0a5941fddc9984b009b8` remains read-only design evidence. Its JUMPI
row at bound 2048 had count-zero gas `85829242`, first-slot count-one gas `85786709`, last-slot
count-one gas `85830087`, positive-count slope about `817.4076`, positive-fit residual/signal about
`0.000486`, and signed activation gap about `+43254.95`. It demonstrates activation/slot-order
contamination but is not a candidate source.

The relation fit reconstructs pure-opcode costs from the frozen rank-98 system and four internal
algebraic basis coordinates; their opcode labels do not give them special physical meaning. `A_i`
and `x_j` use actual per-key raw-gas totals, not execution counts. Calibration is deliberately
staged: within-family count slopes first solve only the two anchor-transfer parameters, then the
reconstructed opcode table is frozen and the four fixed/base costs are fitted from residual block
cost. This prevents the roughly 159M startup term from compensating for the much smaller opcode
signal in one joint least-squares solve. Dynamic raw-gas scenarios are holdouts and never refit the
model.

The commands below are retained as exploratory predecessor workflows only. They are not the current
candidate path, and the old early-STOP opcode fit is explicitly non-candidate.

Generate sealed controlled case metadata and lab inputs:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate \
  --manifest experiments/opcode-gas/manifests/<controlled>.toml \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --out experiments/opcode-gas/runs/<calibration-id>/fixtures
```

Run existing `guest-input.json` cases with a prebuilt launcher:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_opcode_lab.elf \
  --precompile-elf crates/guests/elf/sp1_precompile_lab.elf \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/<controlled>.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/raw-runs.jsonl
```

Run the same opcode fixtures through the revm-backed SP1 guest:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures \
  --guest-launcher target/release/guest-launcher \
  --opcode-stage revm-opcode-lab \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/<controlled>.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/revm-raw-runs.jsonl
```

Use this `revm-opcode-lab` path as the primary opcode-tuning path once the smoke suite is stable. It
executes the generated bytecode through revm's transaction execution/interpreter stack, rather than
the experiment-only mini interpreter used by `opcode-lab`.

### Fast SP1 Opcode Execution

The experiment runner selects SP1's `gas-estimator` execution engine for both `opcode-lab` and
`revm-opcode-lab`. This executes the real SP1 program with `GasEstimatingVM` and returns normalized
`ExecutionReport::gas()`; it is not a fitted formula or a replacement metric. The runner freezes the
canonical gas-estimation cadence (`gas_trace_chunk_threshold = 134217728`,
`gas_trace_chunk_slots = 2`) in calibration provenance and rejects rows produced by another engine
or cadence.

The fast engine is restricted to local SP1 execute-mode opcode labs and production proposal-guest
validation. Proposal gas estimation always loads the production proposal ELF and rejects proof,
network-prover, aggregation, single-ELF override modes, and `RAIKO2_GUEST_ELF_DIR`. Its report
records the exact proposal ELF and guest-launcher SHA-256 digests; `run-proposal` independently
hashes both files and rejects a mismatched report. Precompile, aggregation, and
proof-generation paths continue to use their normal engines. Use direct
`guest-launcher --sp1-execution-engine standard` runs only for focused parity checks; do not mix
standard and gas-estimator rows in one fit. The estimator is an offline calibration and validation
oracle, not a production online quote or admission path. Production online behavior continues to
use the host-native operation ledger and sealed coefficient table.

### Matched-Control Opcode Diagnostics

Matched-control diagnostics keep the bytecode footprint, transaction gas limit, operand setup, and
final stack height equal between target and control lanes. They are useful for finding fixture and
operand-path problems before running the formal controlled candidate workflow:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-calibration \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --guest-launcher target/release/guest-launcher \
  --out experiments/opcode-gas

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate-matched-control \
  --manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --out experiments/opcode-gas/runs/<calibration-id>/fixtures/matched-small-nonzero \
  --operand-profile small_nonzero \
  --generator-max-count 32 \
  --case add --case div --case eq --case exp

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-matched-control \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures/matched-small-nonzero \
  --guest-launcher target/release/guest-launcher \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/matched-small-nonzero.jsonl
```

The `zero` operand profile is the compatibility/default profile: binary operations use `[0, 0]`,
EXP uses `[2, 2]`, and unary operations use `[1]`. The `small_nonzero` profile uses `[7, 3]`,
`[3, 5]`, and `[7]`, respectively. The profile, resolved operands, target/control bytecode, fixture
hashes, and execution identity are all bound into each pair. Validation reconstructs the exact
GuestInput bytecode rather than trusting metadata labels.

These reports contain contextual relative signals. Binary and EXP cases currently report
`OP - POP`. ISZERO and CLZ use one-op SWAP1 controls with the same two-item stack shape and final
stack height, while other unary cases report `OP - NOT`; NOT's self-control delta is intentionally zero. The
`zero` profile additionally supports `MLOAD - NOT`, `KECCAK32 - POP`, `DUPn - DUP1`,
`SWAPn - SWAP1`, and `PC/MSIZE/GAS - PUSH0`; PUSH0 is its own self-control. DUP1, SWAP1, and PUSH0
self-control pairs also intentionally have identical target/control bytecode and zero delta. These
additional families reject `small_nonzero` because they do not define a second operand profile. The
`zero` profile also supports `ADDMOD/MULMOD - (POP+ADD)`, `POP - (NOT+POP)`,
`MSTORE/MSTORE8 - (ADD+POP)`, `MCOPY - (ADD+MUL+POP)`, `PUSHn - PUSH0`, `JUMP - POP`,
`JUMPI - (ADD+POP)`, and `JUMPDEST - (PUSH0+POP)`. Compound controls bind the exact executed target
and reference programs, padding, common suffix, opcode counts, and raw-gas total into the pair.
Coefficients from an early-STOP control are explicitly rejected: equal serialized size alone does
not establish an equal executed workload. These numbers remain contextual relative signals, not
absolute opcode costs, and cannot be copied into a production multiplier table.
Different operand profiles may exercise materially different guest paths: in the initial comparison,
EQ, signed comparisons, division/modulo, EXP, and LT/GT changed between zero and small-nonzero
profiles. A measurement key that needs multiple scenarios must satisfy the formal required-case
rules; a successful diagnostic scenario cannot be generalized to rejected or unmeasured siblings.

Never continue an existing calibration directory after implementation, manifest, dependency, or
guest-artifact changes. Preserve the old run and use `prepare-calibration` at the new clean revision
to create a new content-addressed identity. See the
[SP1 opcode proverGas calibration pitfalls][opcode-calibration-pitfalls] for failure modes and
recovery rules.

[opcode-calibration-pitfalls]: ../../docs/solutions/experiment-correctness/sp1-opcode-prover-gas-calibration.md

### Exploratory Predecessor: Matched Controls And Sequential Overheads

The following legacy matched-control and sequential-overhead commands are exploratory evidence only;
they must not create, repair, or seal the relative-relation/block-fit candidate. Each round
regenerates and reruns one complete frozen footprint; resume verifies the sealed round decisions and
never mixes rows from different generator maxima:

If a required primary overhead has not passed, every dependent overhead is recorded as rejected with
`unmeasured_overhead_dependency` and the missing dependency IDs. The round expands to the next frozen
footprint instead of attempting residualization or aborting; secondary dependency status remains an
independent `unresidualized_dependencies` diagnostic.

Operation workloads retain the frozen generator rounds through 2048. Production-proposal overhead
workloads stop at `overhead_generator_max_count = 128` (at most 129 synthetic blocks). The later
operation rounds reuse the sealed 128 overhead raw file and refit those rows against their current
operation coefficients; they do not rerun or relabel overhead raw data. Decisions and fit artifacts
persist both generator bounds, and resume validation rejects a changed bound or a later operation
record that does not reference the exact sealed 128 raw artifact.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-controlled \
  --fixtures experiments/opcode-gas/runs/<calibration-id>/fixtures \
  --guest-launcher target/release/guest-launcher \
  --calibration-run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/controlled-runs.jsonl

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-controlled-costs \
  --runs experiments/opcode-gas/runs/<calibration-id>/controlled-runs.jsonl \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --out experiments/opcode-gas/runs/<calibration-id>/controlled-fit.json

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-candidate \
  --run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --relations experiments/opcode-gas/runs/<calibration-id>/opcode-relations.json \
  --anchor-probe experiments/opcode-gas/runs/<calibration-id>/anchor-probe-fit.json \
  --block-calibration experiments/opcode-gas/runs/<calibration-id>/block-calibration.json \
  --controlled-fit experiments/opcode-gas/runs/<calibration-id>/controlled-fit.json \
  --provenance experiments/opcode-gas/runs/<calibration-id>/provenance.json

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py build-sp1-bridge \
  --run experiments/opcode-gas/runs/<calibration-id> \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --samples experiments/opcode-gas/runs/<calibration-id>/samples/controlled-cycle-cost-samples.json
```

`run-controlled` executes every overhead fixture through the production `sp1-shasta-proposal`
guest, records three repeats, and emits raw primary and instruction-count observations. Candidate
and bridge construction consume marginal same-unit pairs only: `c_p/c_s` or `f_p/f_s` for operations
and `o_p/o_s` for overheads. Missing or unresidualized secondary dependencies remain explicit
`unavailable` samples and produce a sealable `insufficient_data` bridge; they never fabricate an
instruction-count coefficient or change the primary candidate.

`build-candidate` accepts only the canonical `controlled-fit.json`, `controlled-overheads.json`, and
`provenance.json` inside the calibration run. It revalidates the contiguous persisted decisions,
requires the terminal decision to be `complete`, and binds their exact hashes plus the frozen
experiment identity into the candidate. `build-sp1-bridge` likewise recomputes the expected marginal
sample artifact from that terminal round and binds its exact hash and candidate digest into the
bridge root; copied or edited accepted-looking JSON is rejected.

Run one real proposal GuestInput through RISC0 execute mode and write a normalized raw JSONL row:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-proposal \
  --guest-launcher target/release/guest-launcher \
  --guest-input /tmp/proposal-170-guest-input.json \
  --proof-type risc0 \
  --case proposal-170 \
  --target-raw-gas 30000000 \
  --out /tmp/raiko2-opcode-gas/risc0-proposal-170.jsonl
```

The RISC0 proposal path uses `guest-launcher --stage proposal --proof-type risc0 --mode execute`
and records `risc0_padded_cycles` as the primary workload metric. It also records
`risc0_user_cycles`, segment count, and segment `po2` histogram. It does not produce a proof and
does not call Boundless or any other network prover.

SP1 `prover_gas` and RISC0 cycle counts are backend-native metrics, not the same unit. The SP1
working heuristic that a 30M Ethereum-gas block maps to roughly 10B `proverGas` must not be applied
to RISC0. For RISC0, use a RISC0-native cycle budget, such as a working 30M Ethereum-gas to 10B-cycle
anchor, and report it separately until a cost or time calibration layer exists. Cross-backend
envelopes should compare normalized workload units, not `max(sp1_prover_gas, risc0_padded_cycles)`.

Fit reports from raw runs. SP1 lab runs default to `prover_gas`; RISC0 raw runs can select a cycle
metric explicitly:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/raw-runs.jsonl \
  --out /tmp/raiko2-opcode-gas/report

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit \
  --runs /tmp/raiko2-opcode-gas/risc0-proposal-runs.jsonl \
  --metric risc0_padded_cycles \
  --out /tmp/raiko2-opcode-gas/risc0-report
```

Compute an eth-limit damage frontier and current-Unzen containment report from fit results:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py damage \
  --fit /tmp/raiko2-opcode-gas/report/fit.json \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --eth-gas-limit 30000000 \
  --zk-gas-limit 100000000 \
  --out /tmp/raiko2-opcode-gas/damage
```

Write the Unzen opcode/precompile coverage inventory:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py inventory \
  --manifest experiments/opcode-gas/manifests/sp1-smoke.toml \
  --out /tmp/raiko2-opcode-gas/inventory
```

The `run` command invokes `target/release/guest-launcher` directly with `--stage opcode-lab`,
`--stage revm-opcode-lab`, or `--stage precompile-lab`, always using `--proof-type sp1 --mode
execute --sp1-prover local`. It batches generated inputs into one launcher process per lab stage
through `--input-list` and `--jsonl-out`, so the expensive SP1 executor startup cost is paid once
per stage instead of once per variant. It does not use `cargo run`, does not submit network proofs,
and does not access live L1/L2 RPC.

The `run-proposal` command invokes `guest-launcher` directly for one real Shasta proposal
`GuestInput`. Use this path for block/proposal-level calibration and cross-zkVM observation. It is
not an opcode coefficient generator by itself because one proposal is one workload sample, not a
controlled target-count sweep. Its `--target-raw-gas` value is report metadata for comparing runs;
it does not mean the proposal itself is exactly one 30M-gas block.

The `damage` command is measurement-only. It answers how much measured workload fits under an Ethereum gas
limit, then shows how much of that surface remains reachable under the current-Unzen multipliers
and a chosen block zk gas limit. Realistic block and app impact is a later input to the same report,
not inferred from the smoke lab.

The `inventory` command is coverage-only. It lists every opcode and precompile entry from the current
Unzen table and marks whether the smoke manifest measures it or which future measurement path is
needed.

For a quick smoke after changing launcher code, `target/debug/guest-launcher` built with
`cargo build -p guest-launcher --features sp1-sdk/profiling` also works. Use the release binary for
longer research loops once it has been rebuilt.

## Current Limitation

The default opcode-lab guest is a small experiment-only bytecode interpreter, not revm and not a
complete EVM. It uses narrow synthetic stack and memory semantics to isolate guest-side opcode
workload. Use it as a fast smoke signal and regression anchor, not as evidence that the measured
slope is the exact cost of real EVM execution.

The `revm-opcode-lab` guest runs the same `OpcodeLabInput.bytecode` through revm with a fixed
Osaka/mainnet benchmark transaction and empty benchmark database. This adds real revm transaction
execution, interpreter dispatch, stack, memory, and gas semantics while still removing
block/proposal noise. It is the current primary candidate for opcode coefficient tuning. Do not
replace it with a lower-level direct interpreter call unless there is a specific measurement bug:
that would drop useful revm execution context. The remaining gap is not interpreter reuse, but
realistic context.

The smoke manifest now expands to every active Unzen opcode that can be isolated without state, environment,
or CALL/CREATE wrapper semantics, including arithmetic, comparison, bitwise, stack, fixed
control-flow, and memory-copy templates.

The precompile-lab guest supports direct body measurements for the active Unzen precompiles with
existing fixed deterministic input templates. CLZ (`0x1e`) is now covered by the Osaka revm opcode
lab; p256 (`0x100`) remains `unsupported_by_experiment`. These are direct body calls, not
`STATICCALL` dispatch measurements.
`STATICCALL` wrapper cost, warm/cold account access, precompile argument sweeps, stateful opcodes,
and full block execution are TODOs for later suites.

Stack, control, memory, and precompile slopes include the setup needed by the lab templates, so use
them as smoke damage signals and regression anchors, not as final consensus coefficients.

The real proposal `GuestInput` path is the calibration layer for full block behavior.

For tiny stack and memory templates, always inspect `fit.json` before using `damage.md`. Low R2 means
the current variant counts are too small or the template has setup/cleanup noise. In that case, rerun
with larger variants or a cleaner template before treating the slope as a candidate coefficient.

## Bounded Higher-Layer Calibration

The higher-layer campaign binds the clean implementation revision, exact manifest, operation
coverage, Osaka core, launcher, and production SP1 proposal ELF/VK. It evaluates round 8 first and
opens rounds 32 and 128 only after a persisted expandable decision. Bounds 8 and 32 remain
expansion rounds because neither contains the complete native-transfer count set
`1, 2, 4, 8, 16, 32, 64, 128`; only a complete set at bound 128 can reach the fixed-cost gate.
The native-transfer coefficient `5017` and materiality budget `0.002` are frozen campaign inputs,
not outputs selected or fitted by this campaign. A passing fixed round reports status
`accepted_with_declared_approximation` with decision `accepted`.

The historical v1 campaign is terminal rejected evidence and must never be resumed. Always prepare
a new identity from the v2 manifests shown below. The overhead runner stops after the decision
ledger; it cannot execute state holdouts, which remain gated on successful creation of the sealed
fixed-cost artifact.

```bash
RUN_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  prepare-higher-layer-calibration \
  --manifest experiments/opcode-gas/manifests/sp1-higher-layer-v2.json \
  --operation-coverage experiments/opcode-gas/manifests/operation-coverage-v2.json \
  --augmented-core experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json \
  --guest-launcher target/release/guest-launcher \
  --elf crates/guests/elf/sp1_shasta_proposal.elf \
  --out experiments/opcode-gas/runs \
  --run-path-file "$RUN_PATH_FILE"
HIGHER_LAYER_RUN="$(<"$RUN_PATH_FILE")"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  run-higher-layer-calibration --run "$HIGHER_LAYER_RUN"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  fit-higher-layer-fixed-costs --run "$HIGHER_LAYER_RUN"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  run-higher-layer-state-holdouts --run "$HIGHER_LAYER_RUN"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  finalize-higher-layer-calibration --run "$HIGHER_LAYER_RUN"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  verify-higher-layer-calibration --run-path-file "$RUN_PATH_FILE"
DERIVATION_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  seal-higher-layer-calibration \
  --run "$HIGHER_LAYER_RUN" \
  --out-root experiments/opcode-gas/derivations \
  --derivation-path-file "$DERIVATION_PATH_FILE"
DERIVATION_PATH="$(<"$DERIVATION_PATH_FILE")"
```

The gated order is mandatory: fit the accepted overhead result and successfully create its fixed-cost
artifact before any state command is allowed; then run all state holdouts, finalize, verify, and only
then seal. A missing or conflicting predecessor is rejected. State holdouts are validation-only and
never modify the fixed-cost artifact, its selected round, or its digest. The sealed content-addressed
directory contains exactly four portable files and can be replayed at the frozen source revision
without the live run directories. Proposal validation and production promotion remain out of scope
for this campaign.

### Context-operation augmentation

The context campaign is a separate adaptive controlled run for `ADDRESS`, `CALLER`, `CALLVALUE`,
`CALLDATALOAD`, `CALLDATASIZE`, and `TIMESTAMP`. It does not modify the historical core or the
production table. It uses generator bounds `8, 32, 128, 512, 2048`; count zero, the fit prefix, and
the checkpoint are fixed before execution, accepted scenarios freeze, and only failed scenarios
advance. Every required sibling must pass before its opcode can be recorded as a measured
provisional model; operation coverage V6 remains blocked until the separate cross-ELF transport is
supported.

Use the repository's existing experiment Python environment and run the two-part compatibility
canary before the campaign. The context input is deliberately isolated in
`sp1_context_opcode_lab.elf`; changing the binary input or public commitment of the legacy guest
invalidates historical relation reuse even when its EVM bytecode and raw gas are unchanged. The
legacy 11 relations therefore execute only on the immutable
`experiments/opcode-gas/artifacts/legacy-revm-v1/sp1_revm_opcode_lab.elf` imported from their
accepted calibration revision. The current `crates/guests/elf/sp1_revm_opcode_lab.elf` remains the
stateful laboratory guest and is not a compatibility substitute. The historical PUSH0/SWAP1
absolute anchor probe executes only on `sp1_opcode_lab.elf`. The controlled campaign executes only
on `sp1_context_opcode_lab.elf`. All canary/campaign guests and the release launcher must belong to
the same calibration identity.

The immutable legacy package contains exactly the historical ELF, its VK, and `provenance.json`.
The provenance binds source calibration `51f71fde68f378842f872fc1`, its full calibration-identity
SHA256, source revision, original artifact paths, and both artifact hashes. All three regular,
non-symlink files enter the new calibration identity. Extra, missing, replaced, or symlinked package
entries fail before execution, and result publication may not write inside the package directory.

The package's guest predates the current binary `OpcodeLabInput` serializer. Its v0 bincode wire
encodes `bytecode` as a lowercase `0x`-prefixed string and has no storage field; sending the current
raw-byte wire makes the frozen guest fail while reading stdin. The compatibility runner therefore
selects the launcher's explicit `frozen_legacy_revm_v0` adapter only after the frozen
contract/path checks pass. The adapter rejects storage inputs and binds the exact bytes sent to the
guest to the report and controlled-trace input hash and length. Do not change the global
`OpcodeLabInput` serializer or use the adapter for current/stateful measurements. A compatibility
row without `opcode_lab_wire = frozen_legacy_revm_v0` is invalid.

The fixed legacy canary is an authorization check, not an adaptive measurement. Never expand its
count bounds, refit it, or substitute the context ELF after a failure. The canary proves only that
the isolated legacy guest still authorizes reuse of the historical table. It does not prove that a
marginal cost measured in the separate context ELF is numerically transportable to the legacy cost
basis. V1 records that cross-ELF transport as `not_evaluated`, keeps the context result
`candidate_eligible=false`, and requires independent transport evidence before any candidate
promotion. Controlled sampling may proceed in that explicitly non-candidate state.

```bash
CONTEXT_RUN="$CALIBRATION_RUN/context-campaign"
CONTEXT_FIXTURES="$CALIBRATION_RUN/context-fixtures"
CONTEXT_CANARY="$CALIBRATION_RUN/context-compatibility"

~/.venv/bin/python experiments/opcode-gas/context_opcode_campaign.py run-compatibility-canary \
  --calibration-run "$CALIBRATION_RUN" \
  --controlled-manifest experiments/opcode-gas/manifests/sp1-calibration-v1.toml \
  --baseline-derivation experiments/opcode-gas/derivations/3e1d97c461cd2ef9a40e6a02 \
  --historical-manifest experiments/opcode-gas/tests/fixtures/historical-core-102/controlled-manifest.toml \
  --historical-anchor-run experiments/opcode-gas/runs/09ebb08d76d3f461086b0cf4 \
  --guest-launcher target/release/guest-launcher \
  --legacy-revm-elf experiments/opcode-gas/artifacts/legacy-revm-v1/sp1_revm_opcode_lab.elf \
  --context-elf crates/guests/elf/sp1_context_opcode_lab.elf \
  --control-opcode-lab-elf crates/guests/elf/sp1_opcode_lab.elf \
  --out "$CONTEXT_CANARY"

~/.venv/bin/python experiments/opcode-gas/context_opcode_campaign.py run-campaign \
  --manifest experiments/opcode-gas/manifests/sp1-context-opcode-v1.json \
  --calibration-run "$CALIBRATION_RUN" \
  --run "$CONTEXT_RUN" --fixtures "$CONTEXT_FIXTURES" \
  --guest-launcher target/release/guest-launcher \
  --context-elf crates/guests/elf/sp1_context_opcode_lab.elf \
  --control-opcode-lab-elf crates/guests/elf/sp1_opcode_lab.elf

~/.venv/bin/python experiments/opcode-gas/context_opcode_campaign.py seal-result \
  --manifest experiments/opcode-gas/manifests/sp1-context-opcode-v1.json \
  --calibration-run "$CALIBRATION_RUN" \
  --run "$CONTEXT_RUN" \
  --corrected-core experiments/opcode-gas/derivations/3fc67063a921182e971e7882/core-opcode-submodel.json \
  --coverage-v5 experiments/opcode-gas/manifests/operation-coverage-v5.json \
  --compatibility-canary "$CONTEXT_CANARY/compatibility-canary.json" \
  --out-root experiments/opcode-gas/derivations
```

The execution command remains bound to the calibration's frozen implementation revision. The seal
command is a later offline derivation: it consumes only the canonical create-only adaptive run,
requires the frozen execution revision to remain available as a local commit, verifies the sealed
calibration manifest/provenance and hash-bound native launcher inputs, and runs from a clean committed
analysis checkout. The result records distinct `execution_revision` and `analysis_revision` values;
fixing the derivation code therefore never relabels or reruns the original SP1 measurements. The seal
also requires the canonical `$CALIBRATION_RUN/context-compatibility/compatibility-canary.json`;
injected test executors are stamped synthetic and cannot be sealed. Schema 2 stores the compact
round/decision/fit ledger in
`adaptive-evidence.json` and every full formal row, including failed scenarios, in the deterministic
`adaptive-rows.jsonl.gz` stream. `result.json` contains only the derived models, scenario reports,
and content-addressed source descriptors; it does not duplicate the row stream or source artifacts.
Directory replay reads at most one round at a time, re-admits and refits every row, reconstructs the
terminal selection, and replays native identities without SP1 execution.

Do not replace this path with `read_bytes().splitlines()` or duplicate the rows inside the result
envelope. The production campaign reached roughly 970 MB of uncompressed adaptive rows, and the old
materialized replay was killed by the kernel after constructing several copies of the same object
graph. The streaming format fixes the row, compressed/uncompressed byte, executor, fit, and metadata
limits before reading, parsing, or hashing their contents; one-sided raw/executor EOF fails
immediately. Portable replay proves the sealed bytes, identities, exact fit graph, and deterministic
compression are self-consistent. It does not independently reauthenticate the origin of recorded
`prover_gas`; that authority comes from the create-only production runner and the immutable frozen
calibration identity, while the later analysis revision identifies the exact sealing algorithm.
The provisional legacy-basis projection is exact as arithmetic:
`stored_b_t = (body_scale * delta_lab + r_control * stored_b_control) / r_target`.
Common dispatch and identical setup/cleanup cancel in `delta_lab`; the composite prediction adds
common dispatch exactly once only after transport has independently established a shared cost basis.
While transport is `not_evaluated`, the stored model basis is
`provisional_legacy_projection_unvalidated_cross_elf_transport`; promotion helpers reject it. A
future supported-transport artifact may derive V6 from V5 for the subset of keys whose complete
sibling family passed. Until then, do not publish V6, reseal production estimators, or run proposal
validation.

## Coverage-Qualified Composite Estimator

Seal the review-only SP1 composite estimator only from a clean committed checkout. The artifact
binds the exact opcode registry, operation coverage ledger, corrected higher-layer replay, sealed
typed-storage result, trace schema, ownership policy, implementation revision, and source hashes.
Sealing is create-only: an existing content-addressed directory is never replaced.

The composite estimator derives a coverage overlay from `operation-coverage-v4.json`: schema 3
promotes only SLOAD/SSTORE through sealed result `64065fa462311bdc1848e9d0`; the historical core
registry remains unchanged. V4 refreshes the exact trace-source hashes after typed-storage tracing.
`operation-coverage-v2.json` remains immutable historical input to the sealed higher-layer
derivation, and V3 remains the immutable pre-storage trace snapshot; do not substitute either one
when replaying its original campaign.

```bash
ESTIMATOR_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  seal-composite-estimator \
  --augmented-core experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json \
  --operation-coverage experiments/opcode-gas/manifests/operation-coverage-v4.json \
  --higher-layer experiments/opcode-gas/derivations/3e4de6eecb5e92aa59a6a4b9 \
  --stateful-result experiments/opcode-gas/derivations/64065fa462311bdc1848e9d0 \
  --out-root experiments/opcode-gas/estimators \
  --estimator-path-file "$ESTIMATOR_PATH_FILE"
ESTIMATOR_PATH="$(<"$ESTIMATOR_PATH_FILE")"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  verify-composite-estimator --estimator "$ESTIMATOR_PATH"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  estimate-composite-trace \
  --estimator "$ESTIMATOR_PATH" \
  --trace proposal-operation-trace.json.gz \
  --sp1-report proposal-sp1-report.json \
  --out proposal-estimate.json
```

`estimate-composite-trace` always emits the modeled subtotal, per-layer contributions, independent
coverage denominators, and exact gaps. With zero gaps it also emits `predicted_prover_gas`, whether
or not an SP1 report was supplied. APE and an `evaluated` validation status require an exact report
join on GuestInput, public output, execution mode, engine, frozen gas-trace chunk configuration,
exit status, gas, and primary metric.
Report-identity mismatches are join diagnostics and do not alter estimator coverage or suppress an
otherwise complete prediction.

Proposal startup is charged once, block base once per block, and transaction base once per started
non-Anchor transaction. The frozen native-transfer approximation applies only to committed native
EOA transfers without operation traces. System and Anchor operations are block-owned. Unmeasured
opcodes or precompiles, confirmed spawn wrappers, missing typed features, uncalibrated dirty SSTORE
no-ops, invalid dirty+cold combinations, partial traces, recovery failures, and parity failures remain
explicit gaps; the estimator has no fallback multiplier. Omit
`--sp1-report` for prediction-only use. Both plain JSON and gzip-compressed trace input are accepted.
When `--out` is supplied, its parent must already be a non-symlink directory and the destination
must not exist. Output is published atomically and create-only; paths inside the estimator directory
or its bound source-artifact directories are rejected.

### Task 4 Integration Smoke Checkpoint

The two frozen integration-smoke proposals are Hoodi `79852` and Mainnet `38261`. They are outside
the final 40-Hoodi/20-Mainnet corpus. Do not substitute another proposal, add either smoke row to the
final corpus, or tune a coefficient from its result. The final corpus remains unopened until this
plumbing change has independent review and the two smoke joins complete.

`prepare-integration-smoke` rejects every other network/proposal pair. Its record and the resulting
`run.jsonl` bind `purpose`, `network`, `proposal_id`, the exact GuestInput fixture digest, and the
derived proposal workload identity. An integration smoke accepts only `--proof-type sp1`; a RISC0
execute run cannot be labelled as this checkpoint.

Run the following only from the reviewed clean revision. The two input files are preflight-produced
`GuestInput` files stored under the ignored smoke directory; acquiring them from RPC is a separate
read-only operation. The `--target-raw-gas 1` arguments are legacy raw-run metadata and do not enter
the composite estimate.

```bash
SMOKE_ROOT=experiments/opcode-gas/runs/task-4-integration-smoke
HOODI_INPUT="$SMOKE_ROOT/inputs/taiko_hoodi-proposal-79852.json"
MAINNET_INPUT="$SMOKE_ROOT/inputs/taiko_mainnet-proposal-38261.json"
mkdir -p "$SMOKE_ROOT/inputs" "$SMOKE_ROOT/hoodi" "$SMOKE_ROOT/mainnet"

cargo build --release -p preflight -p guest-launcher --features sp1-sdk/profiling

ESTIMATOR_PATH_FILE="$(mktemp)"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  seal-composite-estimator \
  --augmented-core experiments/opcode-gas/derivations/f945e67bb2c38c9c8ef50530/core-opcode-submodel.json \
  --operation-coverage experiments/opcode-gas/manifests/operation-coverage-v4.json \
  --higher-layer experiments/opcode-gas/derivations/3e4de6eecb5e92aa59a6a4b9 \
  --stateful-result experiments/opcode-gas/derivations/64065fa462311bdc1848e9d0 \
  --out-root experiments/opcode-gas/estimators \
  --estimator-path-file "$ESTIMATOR_PATH_FILE"
ESTIMATOR_PATH="$(<"$ESTIMATOR_PATH_FILE")"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py \
  verify-composite-estimator --estimator "$ESTIMATOR_PATH"

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-integration-smoke \
  --network taiko_hoodi --proposal-id 79852 --guest-input "$HOODI_INPUT" \
  --out "$SMOKE_ROOT/hoodi/record.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-proposal \
  --guest-launcher target/release/guest-launcher --guest-input "$HOODI_INPUT" \
  --proof-type sp1 --case integration-smoke-hoodi-79852 \
  --target-raw-gas 1 --purpose integration_smoke \
  --network taiko_hoodi --proposal-id 79852 \
  --smoke-record "$SMOKE_ROOT/hoodi/record.json" \
  --out "$SMOKE_ROOT/hoodi/run.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py estimate-composite-trace \
  --estimator "$ESTIMATOR_PATH" \
  --trace "$SMOKE_ROOT/hoodi/run.proposal-trace.json.gz" \
  --sp1-report "$SMOKE_ROOT/hoodi/run.guest-launcher.json" \
  --out "$SMOKE_ROOT/hoodi/estimate.json"

~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-integration-smoke \
  --network taiko_mainnet --proposal-id 38261 --guest-input "$MAINNET_INPUT" \
  --out "$SMOKE_ROOT/mainnet/record.json"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-proposal \
  --guest-launcher target/release/guest-launcher --guest-input "$MAINNET_INPUT" \
  --proof-type sp1 --case integration-smoke-mainnet-38261 \
  --target-raw-gas 1 --purpose integration_smoke \
  --network taiko_mainnet --proposal-id 38261 \
  --smoke-record "$SMOKE_ROOT/mainnet/record.json" \
  --out "$SMOKE_ROOT/mainnet/run.jsonl"
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py estimate-composite-trace \
  --estimator "$ESTIMATOR_PATH" \
  --trace "$SMOKE_ROOT/mainnet/run.proposal-trace.json.gz" \
  --sp1-report "$SMOKE_ROOT/mainnet/run.guest-launcher.json" \
  --out "$SMOKE_ROOT/mainnet/estimate.json"
```

For each `estimate.json`, record `predicted_prover_gas`, actual `proverGas`, APE, every layer
contribution, all coverage denominators, and every gap. A smoke result validates only the trace,
identity join, feature extraction, and estimator execution. It cannot modify the sealed estimator,
and it does not authorize acquisition or publication of the final corpus.

## Follow-Up TODO

## Frozen SP1 Calibration Inputs

The controlled SP1 candidate is review-only. Relation-matrix construction, rank validation,
anchor reduction, `mu_zero`, and `B` use exact `Fraction` algebra. Observations, fitting,
prediction, error computation, and canonical numeric serialization use 80-digit `Decimal` values;
binary `float` is never a canonical intermediate. The candidate never writes a protocol multiplier
table, Alethia source, generated guest artifact, or production prover config.
Candidate sealing requires the accepted rank-98 relation system, positive reconstructed ADD,
dynamic raw-gas holdouts, the staged rank-two transfer and rank-four fixed/base fits and holdouts,
and canonical component hashes. The
instruction-count sample, controlled SP1 bridge, and diagnostic overheads have
independent digests and cannot change the primary candidate.

The component ledger keeps unlike units separate:

- measured non-Anchor transaction-phase ordinary opcodes (including EVM `SHA3`) use interpreter raw
  gas, the current schedule multiplier, marginal SP1 `proverGas / raw gas`, and
  ADD-normalized cost index;
- direct precompiles use native gas and the same raw-gas/index convention;
- confirmed CALL/CREATE dispatches use a fixed `proverGas / event` value and
  are never normalized by ADD or forwarded gas;
- startup, block, started-non-Anchor-transaction, and native-transfer costs use their
  declared fixed/base unit, with `N/A` when the current zkGas schedule has no
  independent charge;
- V1 `block_base` owns the complete system/Anchor work and its fixed per-block
  MPT, trie, Merkle, and host-hash baseline. `BLOCKHASH` is not a measured V1
  operation coefficient. Witness/input/blob/KZG counts, unique accesses, and
  dirty-state entries remain diagnostic/unmeasured and never enter `Q_formula`,
  the candidate, or the bridge.

Host/guest internal actions are not silently folded into an opcode raw-gas
index. The V1 block residual deliberately includes system/Anchor and fixed
trie/hash work as one block-level unit, while the execution sum includes only
non-Anchor transaction operations. The same ownership guard is applied to
controlled and proposal rows, so the two terms cannot double count. A future V2
may split these actions after defining independently controlled units; proposal
rows cannot fit or repair them.

Every variable-count case runs three identical local SP1 executions. A
grow-with-count bytecode template is `confounded_template`: bytecode/input
length and all non-target executed counts/raw gas must stay fixed while only
the selected target count changes. Direct precompile measurements additionally
require target/control lanes with identical loop, input, output-length, and
folding shapes. The first passing cumulative sweep is frozen before its mapped
out-of-fit checkpoint; checkpoint data never enters OLS.

`prepare-calibration` freezes a clean implementation revision and the controlled
manifest before measurement. It creates an immutable bridge manifest alongside
the calibration directory; the bridge may later be sealed as `insufficient_data`
without changing the primary `proverGas` candidate.

The calibration identity also preserves the version axes separately: Taiko fork
`Unzen`, production schedule `UNZEN_ZK_GAS_SCHEDULE`, Ethereum upgrade vocabulary
`Fusaka`, REVM execution spec `OSAKA`, proving backend `sp1`, and primary metric
`proverGas`. The exporter derives `OSAKA` from the same `TaikoFork::Unzen` mapping
used by runtime chain selection. Changing or omitting any axis changes the
calibration ID and rejects execution against that frozen run.

`prepare-corpus` selects the fixed 60-row V1 validation corpus (40 Hoodi, 20
Mainnet) from the two checked-in 2026-09-02 fixture files before it contacts an
RPC endpoint. It accepts only network-qualified RPC, chain-spec hash, and
chain-spec file inputs. Each chain-spec file is the JSON list format consumed by
preflight, must contain exactly one selected network entry with an explicit
`hard_forks.UNZEN` activation, and is passed unchanged to preflight. The command
uses the fixed proposal IDs with discovery-only mode and invokes preflight with
the valueless `--validate` flag. It never substitutes a failed proposal. The
saved GuestInputs are written below the ignored repository-relative corpus path
and packed with deterministic tar metadata.

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-corpus \
  --corpus-root experiments/opcode-gas/corpora/sp1-mainnet-hoodi-v1 \
  --manifest experiments/opcode-gas/manifests/proposals/sp1-mainnet-hoodi-v1.json \
  --l1-rpc taiko_hoodi=https://l1.example.invalid \
  --l1-rpc taiko_mainnet=https://l1.example.invalid \
  --l2-rpc taiko_hoodi=https://l2.example.invalid \
  --l2-rpc taiko_mainnet=https://l2.example.invalid \
  --chain-spec-hash taiko_hoodi=<sha256> \
  --chain-spec-hash taiko_mainnet=<sha256> \
  --chain-spec-file taiko_hoodi=config/frozen-taiko-chain-specs.json \
  --chain-spec-file taiko_mainnet=config/frozen-taiko-chain-specs.json
```

The actual final acquisition belongs only to the later measurement step. To
freeze an already-created archive, `publish-corpus` requires a single
content-addressed `gs://.../<archive_sha256>.tar` destination. It uses GCS
create-only upload, reads back the exact object generation, downloads that
generation, and checks the archive hash before recording the URI, generation,
and size in the manifest. `local_unpublished` corpora cannot enter validation.

`prepare-validation` requires the independently sealed candidate and bridge
digests, current `HEAD` equal to their shared `implementation_revision`, and a
fully published 60-row corpus. It writes only below its new validation directory.
Generated corpus, run, manifest, and validation output paths may remain dirty;
any implementation, dependency, controlled-manifest, or guest-artifact change
rejects the operation.

`prepare-calibration` copies and seals the exact controlled TOML under its new
calibration run. Controlled `generate` and `run` commands require both
`--calibration-run` and `--controlled-manifest`; they reject a swapped manifest
or generated fixture whose calibration/controlled-manifest provenance does not
match that run before starting guest execution. Repository-relative paths in
these commands are resolved from the repository root and cannot escape it.

For an `integration_smoke` proposal execution, first write the smoke record with
`prepare-integration-smoke --guest-input <GuestInput.json> --out <record.json>`,
then supply the same `--network`, `--proposal-id`, and
`--smoke-record <record.json>` to
`run-proposal --purpose integration_smoke`. The command validates that the
record is still purpose-labelled `integration_smoke`, is disjoint from the
frozen final 60 rows, is one of Hoodi `79852` or Mainnet `38261`, and binds the exact GuestInput
bytes, derived workload identity, and embedded network/proposal identity before invoking
guest-launcher. The normalized JSONL row preserves all five fields so later review can audit that
the row never entered final validation.

- Add a Taiko/reth-context revm lab that keeps the `revm-opcode-lab` execution path but uses Taiko
  fork config, block env, and realistic tx env instead of fixed Osaka/mainnet benchmark defaults.
- Add stateful benchmark databases for account/storage opcodes, including warm/cold dimensions and
  representative account/storage distributions.
- Add CALL/CREATE wrapper scenarios that separate spawn/no-spawn estimates, call depth, value
  transfer, return-data copying, and precompile dispatch.
- Add `STATICCALL` precompile wrapper measurements on top of the direct precompile body lab.
- Add real proposal/app attribution so damage reports can show normal-workload `zk_util` percentiles
  and top opcode/precompile contributors, not only synthetic attack surfaces.
- Keep SP1 `prover_gas` and RISC0 cycle budgets separate until a calibrated common cost unit exists.

### Abandoned calibration incident: `cbc3b94abe5a18e53fc75f13`

This directory is preserved as abandoned operational evidence, not a model or
proposal conclusion. During the generator-max-128 relation round, the sealed
ledger recorded raw SHA256
`678f57f4d8714a0eb8964248fe783d6a566a89ffec7d4993335117993a6b75cd`, but a
concurrent writer later overwrote the raw file with SHA256
`7c7b1b29cd56bf5f73baede14feebe982c3dc03d8ea55a7a4f80dfb63f5d1b36`.
The round result and ledger otherwise remain sealed, but their source hash
mismatch makes the adaptive campaign invalid and unrecoverable through the
normal resume command. No model was built, no production table or configuration
changed, and no proposal row was opened.
