# RISC0 ZK Gas to Cycle Estimation Experiment

## Goal

Determine whether post-Unzen proposal zkGas can safely replace the local RISC0 pre-execution used
to size Boundless proposal orders.

The experiment measures the current RISC0 proposal guest only. It does not estimate aggregation
cycles, change the protocol zkGas schedule, or change production quoting behavior.

> **Production-policy amendment (2026-08-31):** The experiment gates below record the original
> conservative study design; they are not the current runtime availability contract. The shipped
> opt-in `estimated` strategy deliberately uses exact Unzen, `execution_po2 >= 20`, non-empty
> witnesses with non-zero zkGas, checked arithmetic, and total zkGas at or below `500_000_000`.
> Network, observed block-count/zkGas rectangles, and runtime ELF/image matching do not gate it.
> Release owners enabling `estimated` accept unmeasured cycle drift across guest pairings and
> extrapolation outside collected rectangles; the empirical 10% budget gates concrete collected
> observations during model publication or refresh, not every future request. Use `evaluated` when
> exact cycles are required. The current source of truth is the estimated-quote design spec,
> `docs/API.md`, and the generated model artifact.

The decision target is the Boundless quote bucket, not the best unconstrained regression score. The
current `raiko_agent` strategy rounds proposal estimates up to 1,000 mcycle steps with a 2,000
mcycle minimum. A useful estimator must avoid underquoting while keeping the resulting bucket close
to the bucket obtained from local execution.

## Decision

Run the experiment in two separate stages:

1. A finite, resumable offline collection job builds the dataset and exits after reaching its
   configured sample count.
2. Only after the offline experiment passes, a later production shadow mode compares predictions
   with the existing pre-execution for a fixed number of live Mainnet proposals.

The offline collector is not a daemon and must not poll forever. Production shadowing belongs in
the raiko2 request path so it can reuse the real pre-execution result instead of duplicating
preflight and guest execution in an external process.

## Relationship to Existing ZK Gas Research

`docs/plans/2026-06-08-zkgas-workload-damage-model-design.md` studies protocol-level workload
containment and adversarial proving cost. This experiment has a narrower operational target:
predict the RISC0 user-cycle limit supplied to one Boundless proposal request.

Do not mix the datasets or their target metrics:

- the damage model compares backend-native workload surfaces and protocol limits;
- this experiment predicts `session.cycles()` for one pinned RISC0 proposal ELF;
- SP1 `proverGas`, RISC0 padded cycles, and aggregation cycles are out of scope.

## Cohort Invariants

Every sample in one model cohort must use exactly the same:

- raiko2 source revision;
- RISC0 SDK version;
- proposal guest ELF and full Boundless image ID;
- RISC0 execution configuration, including `execution_po2`;
- Unzen zkGas schedule.

Build or select the proposal ELF once before collection and record its image ID in the run manifest.
The collector must verify the requested source revision against the repository HEAD and the requested
RISC0 version against `Cargo.lock`. Hash the collector, proposal ELF, preflight and guest-launcher
binaries, discovery script, lockfile, and resolved chain specification into the run manifest and each
sample row. If any invariant or artifact hash changes, stop the run and start a new cohort. Never
combine samples across image IDs and compensate with a regression feature.

## Network and Fork Scope

Use only Taiko Hoodi and Taiko Mainnet:

- Hoodi Unzen activation: `2026-06-18 13:00:00 UTC` (`1781787600`).
- Mainnet Unzen activation: `2026-08-06 13:00:00 UTC` (`1786021200`).

A proposal is eligible only when every included L2 block:

- has a timestamp at or after the network's Unzen activation;
- has non-zero header `difficulty`;
- is represented in the exact `GuestInput` used for RISC0 execution.

Exclude Masaya, devnet, every pre-Unzen proposal, every proposal crossing the activation boundary,
aggregation inputs, and historical cycle results produced by a different image ID.

The fork timestamp from the chain specification is the eligibility source of truth. Non-zero
`difficulty` is an additional integrity check, not a replacement for fork detection.

## Sampling Design

Collect one row per proposal, not one row per block.

The initial offline run contains:

- 120 Hoodi proposals:
  - 80 fitting samples;
  - 40 calibration samples used only to derive the one-sided safety residual.
- 60 Mainnet proposals held back as the final cross-network test set.

Do not select only consecutive tip proposals. Before running preflight, scan eligible proposal spans
and their L2 headers, then stratify candidates across the joint distribution of:

- total proposal zkGas;
- proposal block count.

Sample across the joint quantiles and deliberately include pairs with:

- similar total zkGas but different block counts;
- similar block counts but different total zkGas;
- low, median, and high total zkGas;
- unusually high transaction count or serialized input size when available.

This prevents the experiment from attributing a block-count correlation to zkGas. If a joint
stratum has too few proposals, record the coverage gap and sample the nearest available candidates
rather than silently duplicating another stratum.

The Mainnet set remains untouched until the model and safety residual have been selected from the
Hoodi fitting and calibration sets. Do not tune coefficients or margins after inspecting Mainnet
results.

## Offline Collector

Implement the collector as a finite command with explicit network, target count, output directory,
and pinned image metadata. It should support separate Hoodi and Mainnet invocations rather than one
unbounded mixed-network loop.

Proposal selection produces a finite candidate manifest before any expensive execution starts. The
collector consumes that fixed manifest and never watches the chain tip for more work. It accepts a
target sample count and a maximum candidate count, then either reaches the target and exits zero or
exhausts the manifest and exits non-zero with the shortfall recorded.

The candidate manifest declares exact per-split targets: Hoodi requires `fit` and `calibration`, and
Mainnet requires `holdout`. Each target is positive, the targets sum to the invocation's total target,
and the candidate prefix selected by the maximum candidate count covers every split quota. Completion
is evaluated per split so excess fitting samples cannot hide a calibration shortfall.

For each selected proposal, the collector:

1. Uses `scripts/regression/stress_shasta_proposal.py --discover-only` to resolve the complete
   proposal tuple.
2. Runs `preflight` once and persists the generated `GuestInput` in the run directory.
3. Rechecks fork eligibility and computes features from that exact `GuestInput`.
4. Runs the pinned RISC0 proposal ELF with `guest-launcher --proof-type risc0 --mode execute`.
5. Appends one result record and fsyncs or atomically replaces the progress manifest.
6. Exits successfully after the requested number of valid samples has been collected.

The collector must be resumable and idempotent by `(network, proposal_id, image_id)`. A rerun skips
completed samples, retries explicitly retryable infrastructure failures, and preserves terminal
preflight or guest failures as result rows instead of silently removing difficult workloads.

Discovery and preflight receive the same resolved chain-specification file. Their cache outputs must
be validated and published with same-directory atomic replacement so a timeout cannot expose a
partial cache entry. Before creating the run directory, require that the resolved list contain
exactly one selected L2 entry and exactly one corresponding L1 entry; otherwise discovery and
preflight could resolve partial or duplicate overrides differently. Append result rows with file
`fsync` and replace progress atomically. On resume, the collector may repair only the final
unterminated JSONL record: truncate a malformed torn tail, or durably append the missing newline
after a complete JSON object. Malformed interior or newline-terminated records remain fatal, and
model fitting never repairs its input.

Use a configurable working directory. Examples may use a temporary path such as
`/tmp/raiko2-risc0-zkgas-cycles`, but the implementation must not embed a user-specific filesystem
path.

## Recorded Data

Persist an append-only JSONL sample file plus a run manifest. Each successful result records at
least:

- network and proposal ID;
- L1 inclusion block, last anchor block, and L2 start/end;
- source revision, image ID, RISC0 version, and `execution_po2`;
- first and last L2 timestamps;
- block count;
- total zkGas, computed as the sum of block header `difficulty`;
- minimum, median, p95, and maximum per-block zkGas;
- transaction count;
- serialized `GuestInput` byte length;
- proposal state-node count and other cheap witness-size measurements available without execution;
- RISC0 user cycles from `session.cycles()`;
- `evaluated_mcycles_count`, using the same ceiling conversion as Boundless;
- the current 1,000-mcycle quote bucket;
- preflight, execution, and wall-clock durations for operational diagnosis.

The authoritative zkGas value comes from
`GuestInput.witnesses[].block.header.difficulty`. The collector may compare it with RPC headers, but
must reject the sample on a mismatch instead of fitting against RPC data that differs from the guest
input.

RISC0 user-cycle counts should be deterministic for a fixed input, ELF, and execution
configuration. Re-run a randomly selected 10 percent of samples as a determinism check; do not
triple-run every sample merely to average wall-clock noise.

## Candidate Models

Evaluate models in increasing complexity:

```text
M1: mcycles = alpha + beta * total_zkgas

M2: mcycles = alpha + beta * total_zkgas
                    + gamma * block_count

M3: mcycles = alpha + beta * total_zkgas
                    + gamma * block_count
                    + delta * guest_input_bytes
```

Start with M1. Adopt M2 only when M1 residuals retain a material block-count relationship. Adopt M3
only when M2 residuals retain a material input-size relationship. Do not add a network coefficient
in the initial model: Mainnet is intended to test whether the Hoodi relationship transfers. A
systematic Mainnet residual requires investigation before adding a chain-specific correction.

Fit coefficients on the 80 Hoodi fitting samples. For the initial experiment, define the Hoodi
calibration margin as the largest positive residual observed in the 40 Hoodi calibration samples:

```text
calibration_margin = max(0, max(actual_mcycles - predicted_mcycles))
```

The candidate production estimate is:

```text
safe_mcycles = model_prediction + calibration_margin
quoted_mcycles = max(2000, ceil_to_multiple(safe_mcycles, 1000))
```

The calibration method and margin must be recorded in the model artifact. Mean regression alone is
not a safe cycle limit because Boundless receives the quote through `with_cycles(...)`; an
underestimate can make the request unfulfillable rather than merely make its price inaccurate.

## Evaluation and Gates

Report ordinary diagnostics such as R-squared, MAE, MAPE, and residual plots, but make the decision
from quote safety and efficiency:

- zero underquotes in the untouched Mainnet holdout;
- at least 90 percent exact quote-bucket matches in the Mainnet holdout;
- p95 quote overhead no greater than 1,000 mcycles;
- no remaining material residual trend against block count, total zkGas, input bytes, or network;
- deterministic cycle counts for every repeated sample;
- all sample exclusions and failures accounted for in the report.

An underquote occurs when `quoted_mcycles < actual evaluated_mcycles_count`. If M1 fails, evaluate
M2; if M2 fails, evaluate M3. If M3 fails, conclude that the available cheap features do not safely
replace pre-execution. Do not relax the safety gate merely to produce a model.

Passing a finite holdout is evidence, not a mathematical guarantee. A production implementation
must also fall back to local execution when the image ID, fork, schedule, or input feature envelope
does not match the calibrated cohort.

## Production Shadow Stage

The shadow stage is a separate follow-up change. Keep the existing local RISC0 pre-execution and,
for each real Mainnet Boundless proposal request:

1. Compute the candidate estimate from the already available `GuestInput`.
2. Run the existing pre-execution and retain its actual cycle count.
3. Record the predicted value, safe value, predicted bucket, actual value, actual bucket, model ID,
   image ID, and fallback reason if any.
4. Preserve the existing actual value for the Boundless request; shadow predictions must not affect
   pricing or the cycle limit.

Run shadow mode for 1,000 consecutive eligible Mainnet proposals produced by one model cohort. The
switch away from pre-execution requires:

- zero observed underquotes;
- the offline efficiency gates to remain satisfied;
- no unexplained time drift in residuals;
- explicit review of every out-of-envelope fallback.

Any image or zkGas schedule change invalidates the cohort and restarts shadow collection.

## Scope Boundary for Removing Pre-Execution

This experiment validates only cycle estimation. The current local execution also returns the guest
journal used to construct the Boundless request and verify the expected input hash. A later change
that actually skips pre-execution must construct and validate the proposal journal through the
deterministic host path and test it independently.

Aggregation has no proposal zkGas mapping and remains on its current fixed, evaluated, or separately
calibrated path.

## Deliverables

The offline experiment should produce:

- a run manifest containing all cohort invariants and sampling parameters;
- cached proposal metadata and `GuestInput` files;
- append-only raw result JSONL;
- a fitted model artifact with coefficients and calibration margin;
- a Markdown report comparing M1, M2, and M3 against the gates;
- an explicit recommendation to proceed to shadow mode or retain pre-execution.

No coefficient should be copied into production configuration directly from an ad hoc notebook or
terminal transcript. The model artifact and its input manifest are the reviewable source of truth.

## Non-Goals

- Do not use Masaya or pre-Unzen proposals.
- Do not combine samples from different proposal image IDs.
- Do not run network proofs or submit Boundless orders during offline collection.
- Do not estimate aggregation cycles from proposal zkGas.
- Do not modify the protocol zkGas table or block zkGas limit.
- Do not remove pre-execution as part of the collector change.
- Do not keep the offline collector running indefinitely.
