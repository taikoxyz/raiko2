# ZKGas Multiplier Recalibration Experiment Design

## Status

Approved design for an offline experiment. This document defines how measurement runs are made and
reported. It does not authorize or implement a production zk gas schedule change.

The earlier SP1 opcode lab, coverage inventory, and workload damage documents remain useful component
designs. This document is the authoritative contract for a complete multiplier recalibration run.

## Goal

Produce a more accurate candidate mapping from EVM work to backend-native zkVM workload, starting
with SP1, so developers can decide whether to update the Unzen zk gas multiplier table in
alethia-reth.

The experiment must answer two separate questions:

1. What marginal SP1 `proverGas` is attributable to each measurable opcode or precompile scenario?
2. After accounting for proposal, block, transaction, witness, and other non-opcode overhead, does a
   candidate multiplier table predict a fixed corpus of Mainnet and Hoodi proposals more accurately
   than the current table?

The result is evidence and a candidate table. A developer separately decides whether to change the
production alethia-reth/REVM schedule.

## Non-Goals

This experiment does not:

- change the production zk gas multiplier table;
- change the production block zk gas limit;
- change Boundless quoting or any online prover path;
- add runtime SDK, ELF, image, fork, network, or model-version checks;
- automatically promote a candidate table;
- automatically open or merge an alethia-reth change;
- submit SP1 or RISC0 proofs to a network prover;
- choose a common raw unit across different zkVMs.

The current 100M per-block zk gas cap is not an experimental fitting constraint. It may be included
as report context, but synthetic generation and coefficient fitting must not reject or truncate a
case because it would exceed that cap.

## Fixed Experiment Contract

Each run is an immutable experiment with four frozen inputs:

1. **Implementation provenance**: exact raiko2 source revision used by the tooling, the Cargo-pinned
   alethia-reth revision, the exported Unzen schedule hash, guest ELF/VK hashes, and relevant SDK
   versions from that checkout.
2. **Backend**: one backend and one native metric. The first production-quality run uses SP1 6.3.0
   and software `proverGas` from the current `main` dependency set.
3. **Synthetic suite**: exact manifest, generated-case metadata, and fixture hashes used for opcode
   and precompile measurement.
4. **Proposal corpus**: an explicit, saved set of Mainnet and Hoodi proposal GuestInputs, identified
   by network, proposal ID, repository-relative fixture path, and SHA256.

The runner must refuse to mix rows whose provenance does not match the run manifest. A different
SDK, dependency revision, ELF, schedule, synthetic manifest, or proposal corpus creates a new run;
it never overwrites or silently extends an existing report.

These checks protect the integrity of offline experimental data only. They are not copied into the
runtime or used to decide whether production may use an installed schedule.

## Backend Independence

Every zkVM is measured independently in its native deterministic execution metric:

- SP1: software `proverGas`;
- RISC0: user and padded cycles plus segment information;
- future zkVMs: that backend's documented deterministic workload metric.

Raw values from different backends are not comparable, so the experiment must not compute
`max(sp1_prover_gas, risc0_cycles, ...)`.

Each backend produces its own cost vector and candidate multiplier table first. A future report may
normalize backend-native values against a shared set of reference workloads. Only after that
normalization may developers choose separate tables or a conservative cross-backend envelope.

V1 implements and reports the SP1 lane first. The data and artifact schema must allow later RISC0
and other backend lanes without changing the meaning of existing SP1 reports.

## Workload Decomposition

Proposal workload is modeled as:

```text
backend_workload =
    proposal_fixed_cost
  + block_count * block_cost
  + transaction_count * transaction_cost
  + sum(opcode_raw_evm_gas_i * opcode_multiplier_i)
  + sum(precompile_feature_j * precompile_cost_j)
  + witness_input_blob_and_other_cost
```

This is a reporting and fitting decomposition, not a new runtime metering formula.

The one-dimensional opcode multiplier table cannot represent every source of zkVM work. The
experiment therefore keeps non-opcode effects visible instead of forcing them into opcode
coefficients. At minimum, proposal validation records:

- proposal and block count;
- transaction count;
- opcode counts and raw EVM gas contribution by opcode;
- precompile calls, native gas, and input-size features when available;
- guest input and witness size features;
- blob or KZG-related features when present;
- actual backend-native workload;
- current-table prediction, candidate-table prediction, and residual overhead.

The first candidate may use an explicit residual or overhead model in the report. Only fields that
fit the current alethia-reth schedule shape are emitted as schedule candidates.

## Measurement Layers

### 1. Active Schedule Export

The experiment reads the current Cargo-pinned
`alethia_reth_evm::zk_gas::unzen::UNZEN_ZK_GAS_SCHEDULE` through the existing xtask exporter. Python
must not contain another hand-maintained copy of the production table.

The exported schedule is both the baseline and the coverage inventory. Its normalized content hash
is stored in the run manifest.

### 2. SP1 Synthetic Microbenchmarks

The primary opcode lane is the revm-backed `revm-opcode-lab`, not the experiment-only mini
interpreter. Direct precompile body measurements use `precompile-lab`. The mini interpreter remains
a fast smoke signal only.

For each supported scenario, generate a matched baseline and an adaptive count sweep. Variants must
keep setup, environment, calldata shape, and cleanup as constant as practical while changing the
target feature count.

Fit the marginal relationship:

```text
sp1_prover_gas = intercept + slope * target_feature_count
```

The sweep expands until it has enough dynamic range for a stable fit or reaches a declared case
limit. Each result records sample counts, slope, intercept, R2, residuals, observed range, and why a
case was skipped or rejected. Low-quality fits remain visible in the report and are not silently
converted into production candidates.

State- and environment-dependent opcodes, CALL/CREATE families, halting behavior, and unsupported
entries use explicit scenarios or explicit unsupported classifications. Precompiles with
argument-dependent cost use deterministic input families and preserve the input dimensions in raw
results.

All SP1 runs use local execute mode. They never request a proof and never call a remote prover.

### 3. Fixed Proposal Feature Extraction

For every saved proposal GuestInput in the corpus, execute the same proposal workload through an
instrumented host/revm path to extract opcode, precompile, block, transaction, witness, input, and
blob features.

Feature extraction and backend measurement are joined by the SHA256 of the exact GuestInput bytes,
not merely by proposal ID. A proposal ID is human-readable provenance; the hash is the experiment
identity.

The extractor must account for every block and transaction in the proposal. It must report
unattributed work instead of dropping it when a feature is unavailable.

### 4. Fixed Proposal SP1 Execution

Execute each exact GuestInput locally with the SP1 proposal guest and record actual `proverGas` plus
the available instruction, syscall, cycle-tracker, and timing diagnostics.

The runner is finite and resumable. Completed rows are content-addressed by run identity and case
identity. On restart it verifies an existing row before skipping it; it does not append duplicate or
mixed-provenance observations.

### 5. Candidate Construction And Validation

Construct high-precision candidate coefficients from accepted microbenchmark fits. Translate them
to the integer schedule representation only as a final report step, with the rounding rule stated
explicitly.

Evaluate the current and candidate tables against the same fixed proposal corpus and the same
overhead model. Report at least:

- signed and absolute error per proposal;
- MAPE and maximum absolute percentage error;
- underprediction count and maximum underprediction;
- results split by Mainnet and Hoodi;
- total and per-component predicted workload;
- the proposals and components responsible for the largest residuals;
- coverage gaps and coefficients excluded for poor fit quality;
- candidate-to-current multiplier deltas.

The report does not need to enforce zero underprediction. It must expose underprediction and the
chosen accuracy tradeoff so developers can decide whether the candidate is useful. A 10% workload
error may be operationally acceptable, but that tolerance is a report interpretation, not a rule
that hides larger errors or automatically promotes the table.

## Fixed Proposal Corpus

The proposal corpus is defined by a checked-in manifest. Every row contains:

```text
network
proposal_id
fixture_path
guest_input_sha256
split
notes
```

`network` is descriptive and is used for split metrics; it does not select network-specific opcode
coefficients. `split` is fixed before results are examined. Supported values are `fit`,
`calibration`, and `holdout`.

Existing GuestInput fixtures are referenced by repository-relative path and are not copied into the
experiment directory. Any newly selected fixture must be saved before the run and hashed in the
manifest. A report is invalid if a fixture is missing or its bytes no longer match the manifest.

The corpus is intentionally selected and finite. The runner does not discover recent proposals from
RPC, replace failures with new proposal IDs, or move a proposal between splits after observing its
result.

## Artifact Layout

```text
experiments/opcode-gas/
  manifests/
    <synthetic-suite>.toml
    proposals/<corpus-id>.json
  runs/
    <run-id>/
      experiment.json
      proposal-manifest.json
      raw/
        sp1-microbench.jsonl
        proposal-features.jsonl
        sp1-proposal-runs.jsonl
      fits/
        sp1-cost-vector.json
        overhead-model.json
      candidate/
        sp1-multipliers.json
        schedule-candidate.rs
      report.json
      report.md
```

The checked-in fixed report is self-contained enough to review without rerunning SP1. It includes
provenance, fixture identities, commands, accepted/rejected fits, model diagnostics, proposal-level
results, and the exact candidate table.

Large transient launcher output and build artifacts remain outside Git. The committed raw JSONL is
the compact normalized observation stream, not terminal logs or generated binaries.

Run IDs are content-derived or otherwise collision-resistant. Generating into an existing run
directory is rejected unless an explicit resume operation proves that all immutable inputs match.

## Proposed Stable Workflow

The finished tooling exposes separate, composable steps rather than one command with hidden
side-effects:

```bash
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py prepare-run ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py generate ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py trace-proposals ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py run-proposals ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py fit-multipliers ...
~/.venv/bin/python experiments/opcode-gas/opcode_gas.py report ...
```

Exact flags are defined in the implementation plan. Every step reads the same immutable
`experiment.json`; later steps fail on missing, mismatched, or duplicate inputs.

Python commands use the user-managed `~/.venv`. The implementation must not create another virtual
environment or install packages implicitly.

## Failure And Resume Semantics

- Invalid synthetic or proposal fixtures fail before measurement.
- Guest execution failures produce explicit failed rows and a non-zero command exit.
- NaN, infinity, malformed metrics, duplicate case identities, and integer overflow reject report
  generation.
- A partial run remains resumable, but it cannot be reported as complete.
- Resume validates environment, manifest, fixture hashes, and existing result schemas before reuse.
- Report generation lists missing and failed cases and refuses a final status unless the declared
  corpus is complete.
- External RPC availability is not required after the proposal fixtures have been saved.

## Production Boundary And Promotion

Experiment code never edits alethia-reth, generated ELF assets, runtime configuration, or Boundless
configuration. It emits a candidate table and a human-readable delta from the active table.

There is deliberately no automatic promotion command. If the report supports an update, a developer
may create a separate alethia-reth change, review the schedule diff, run that repository's tests, and
then update raiko2's alethia-reth revision through the normal dependency workflow.

Deployment owners decide when a new schedule is suitable for a particular release. Production code
does not inspect this experiment's run ID, SDK version, ELF hash, proposal corpus, or report before
using the schedule compiled into that release.

## Verification

The implementation is complete when:

- the active schedule export round-trips without a Python pricing-table copy;
- the full inventory has no unclassified opcode or precompile entries;
- synthetic generation is deterministic for a fixed manifest;
- a run cannot mix SDK, ELF, schedule, manifest, or fixture provenance;
- resume does not duplicate completed cases;
- SP1 execution is local execute-only;
- fixed Mainnet and Hoodi proposal rows join by GuestInput hash;
- the report reproduces both current-table and candidate-table metrics from committed normalized
  observations;
- missing, modified, failed, or duplicate fixtures prevent a final report;
- no experiment command modifies the production table or runtime configuration;
- tests cover deterministic artifacts, provenance mismatch, resume, report recomputation, integer
  conversion, and incomplete-corpus failures.

## Future Extensions

- Add RISC0 opcode/proposal measurements as an independent cycle-based lane.
- Add other zkVM-native metrics using the same feature schema.
- Add shared reference workloads for cross-backend normalization.
- Add multi-dimensional state, memory, CALL/CREATE, and precompile scenarios if the production
  schedule evolves beyond one multiplier per entry.
- Add more fixed proposal corpora as new immutable runs rather than changing historical reports.
