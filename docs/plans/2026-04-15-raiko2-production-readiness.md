# Raiko2 Production Readiness Assessment

Date: 2026-04-15
Status: Working assessment

## Executive Summary

`raiko2` is no longer blocked on core proving capability. The current tree has recent live
evidence that:

- latest-proposal `SP1` reserved-network proving completed successfully
- latest-proposal `RISC0` Boundless proving completed successfully
- `SP1` aggregation from externally supplied proofs completed successfully
- latest-proposal Shasta preflight has been reduced to a practical latency envelope

However, the project is **not yet production ready** under a strict operator-facing definition.
The remaining work is not “make proving possible”; it is “make the service safe to operate as the
single canonical production path.”

Baseline estimate from the current state: **about 1 week of engineering time**.

This estimate is intentionally strict:

- unmerged local changes do **not** count as done
- recent live proof samples count as evidence of capability, not evidence of production readiness
- docs/config/readiness/runtime semantics must be aligned before the service should be called
  production ready

## Current Verified State

### 1. Proposal proving is live for both proving backends

Recent successful live samples exist for the latest proposal path:

- `SP1` latest proposal `2670` completed with proof present
  - evidence: `target/compare/latest-proposal2670-sp1-network-status-poll-2.json`
  - key fields:
    - `status = completed`
    - `provider_request_id = 0x3601a990fcea3594f3967fc90d683a896381148938518c19860528528341b5c8`
    - `proof_present = true`
    - `proof_len = 7624`
- `RISC0` latest proposal `2670` completed through Boundless with proof present
  - evidence: `target/compare/latest-proposal2670-risc0-boundless-oldquote-status-final.json`
  - key fields:
    - `status = completed`
    - `provider_request_id = 0x382bba7d7bc9ae86c5de3e16c4ca96bcc0a3478e2d4b3048`
    - `evaluated_mcycles_count = 1188`
    - `quoted_mcycles_count = 2000`
    - `proof_present = true`

### 2. External-proof aggregation works

`SP1` aggregation is no longer blocked on the internal state-machine dependency chain. External
proof objects can be aggregated successfully.

- evidence:
  - `target/compare/latest-proposals2669-2670-sp1-aggregate-proof-network-vkjson.json`
  - `target/compare/latest-proposals2669-2670-sp1-aggregate-report-network-vkjson.json`
- key fields from the report:
  - `stage = aggregation`
  - `mode = prove`
  - `proof_mode = plonk`
  - `public_values = 0xf079cd1e8d276f270dfe29cbb67fddcce23f46758d5db7fa09a8eca0b739266f`
  - `wall_time_ms = 97410`

### 3. Latest-proposal preflight is materially faster than the historical regression case

The current preflight path is no longer in the “minutes for one recent proposal” range. Recent
latest-proposal measurements are practical for live use.

- evidence:
  - `target/compare/latest-proposal2670-preflight.json`
  - `target/compare/latest-proposal2670-risc0-boundless-evaluated-logs.txt`
- verified recent sample:
  - `proposal_id = 2670`
  - `block_count = 192`
  - `elapsed_ms = 18687`
- runtime logs show proposal-window fetching working as intended:
  - `debug_executionWitness` fetched in proposal windows
  - account proofs fetched in proposal windows
  - chunked preflight completes across the full proposal range

### 4. Current service binaries still compile on the main path

The current working tree is large, but the main service path still compiles:

- `cargo check -p raiko2 -p raiko2-prover -p raiko2-provider --locked`
- `cargo check -p preflight -p witness-check --locked`

This is necessary, but not sufficient, for readiness.

### 5. Readiness, task-status semantics, and public config/docs have materially improved

The most obvious operator-facing control-plane gaps are no longer in their earlier state.

Recent hardening completed in the current tree:

- `GET /ready` now checks:
  - configured L1 and L2 RPC chain IDs for each allowed pair
  - queue backend readiness
  - prerequisite readiness for the configured default proving route
- `GET /v3/tasks` is now read-only and no longer mutates runtime state while serving queries
- root task runtime status is derived consistently from the summarized task state instead of
  mixing stale stored values with query-time side effects
- public docs and config examples now reflect current live defaults, including:
  - Boundless batch quote controls
  - current SP1 operator settings
  - `zk_any` sampling on `/v3/proof/batch/shasta`

Relevant code and docs:

- `bin/raiko2/src/server/ready.rs`
- `bin/raiko2/src/server/handlers/proof.rs`
- `config.example.toml`
- `docs/API.md`
- `docs/operations.md`

## Why This Is Not Yet Production Ready

### 1. Runtime and task status semantics are improved, but not yet fully canonical

The most dangerous query-time side effects are gone, but the system still does not have a single
fully authoritative operator-facing task-state model.

What is better now:

- `/v3/tasks` no longer writes back into runtime storage
- root response status is derived more consistently
- cancellation behavior for shared underlying engine tasks is safer

What is still missing:

- one explicit canonical task-state contract across engine state, runtime observer state, and
  route-specific metadata
- clearer operator guarantees about which state fields are authoritative during retries, external
  proof submission, and partial failure scenarios

Relevant code areas:

- `bin/raiko2/src/server/handlers/proof.rs`
- `bin/raiko2/src/server/state/runtime_observer.rs`
- `crates/runtime/src/lib.rs`

Production implication:

- operators can reason about the system much better than before, but there is still room for
  ambiguity under edge conditions
- support/debug workflows are still more expensive than they should be

### 2. SP1 live success still depends on a reduced verification posture

Recent successful SP1 live runs used:

- `verify = false`
- `network_mode = reserved`
- `fulfillment_strategy = reserved`

Evidence:

- `target/compare/latest-proposal2670-sp1-network-request.json`

That proves the proposal proving path works, but it does **not** yet prove the final production
verification posture is fully locked.

### 3. Aggregate route behavior is better, but not yet fully production-closed

The underlying aggregation logic works, and the hosted route is now in a much better place than
when this assessment started.

What is verified:

- external-proof aggregation works successfully underneath
- `/v3/proof/aggregate` accepts canonical external proofs on the hosted route
- request body handling is explicitly aligned with old `raiko` API limits

What is not yet proven to production standard:

- full live hosted-route success from the canonical deployed service, not just fixture or offline
  paths
- final operator guidance around payload expectations and failure handling for real external proofs

### 4. The service still depends on an external Taiko L2 endpoint for the working production path

Recent successful latest-proposal runs rely on:

- `http://<l2-rpc-host>:8545` (literal host is internal-runbook only)

The alternative “old-style local input-provider endpoint” route was investigated and is not
currently a usable production fallback.

Operational consequence:

- production readiness still depends on a single external L2 witness-capable endpoint being healthy
- endpoint strategy, fallback policy, and operational ownership are not yet fully documented

### 5. The current work is still sitting in an unmerged, multi-area diff

Current local diff touches proving, provider, pipeline, queue/runtime, guest build tooling, guest
ELFs, and docs/config surfaces.

`git diff --stat` currently shows a large multi-area change set. Under a strict readiness
definition, none of this counts as done until it is:

- reviewed
- merged
- regressed
- deployed
- revalidated on live routes

## What Changed Since The Historical Readiness Roadmap

The existing roadmap in
`docs/plans/2026-02-03-raiko2-readiness-roadmap.md`
is no longer an accurate statement of the main blockers.

That document still treats these as primary gaps:

- proposal derivation
- config-injected Shasta proposal sourcing
- early deployment wiring gaps

Those are no longer the practical center of risk.

The current blocker profile is instead:

- task/runtime semantics still need final canonicalization
- SP1 verification posture is not fully closed
- hosted aggregate route still needs final live proof-path validation
- external endpoint strategy and ownership still need closure

In short:

- **old blocker:** “can the system prove?”
- **current blocker:** “is the validated branch-ready capability also operationally hardened enough
  to be the single production path?”

## Baseline ETA: 1 Week

This is a **single baseline estimate**, not an optimistic target.

### Work package A — Merge and normalize the current tree (1 to 2 days)

Required outcomes:

- split or otherwise make the current diff reviewable
- ensure the readiness assessment itself does not drift from repo truth

Why it matters:

- without this, the repo has capability but no trustworthy operator-facing contract

### Work package B — Finalize runtime/task-state semantics (1 to 2 days)

Required outcomes:

- define one authoritative task-state story for operators
- make `/v3/tasks` output consistent and predictable

Why it matters:

- this is the difference between “works in successful samples” and “safe to operate”

### Work package C — Close the remaining proving-route product gaps (2 to 3 days)

Required outcomes:

- lock the intended SP1 verification posture
- finish aggregate route production closure at the HTTP surface
- validate Boundless quote behavior and failure modes with the chosen policy

Why it matters:

- recent samples prove capability, but not complete route-hardening

### Work package D — Regression, runbook, and release validation (2 to 3 days)

Required outcomes:

- run the relevant focused checks and service-level regressions
- document operator runbooks and incident expectations
- re-run live success samples after merge/deploy from the canonical path

Why it matters:

- production readiness is a merged-and-reproducible property, not a branch-local property

## Exit Criteria

`raiko2` should only be called production ready when all of the following are true:

1. **Proposal proving succeeds on the canonical live path**
   - latest-proposal `SP1` live proof succeeds
   - latest-proposal `RISC0` live proof succeeds

2. **Aggregation succeeds on the canonical live path**
   - hosted aggregate route works for real canonical proof payloads

3. **Readiness and task-state semantics are trustworthy**
   - `/ready` covers actual proving dependencies
   - `/v3/tasks` no longer presents contradictory or weakly-authoritative states

4. **Docs and config are aligned**
   - `docs/API.md`
   - `docs/operations.md`
   - `config.example.toml`
   all match live behavior

5. **The current branch state is no longer branch-local**
   - merged
   - regression-tested
   - rolled out
   - revalidated in the deployed environment

## Recommendation

Do **not** describe `raiko2` as production ready today.

A more accurate statement is:

> `raiko2` has validated end-to-end proving capability on the primary latest-proposal paths for
> both `SP1` and `RISC0`, and external-proof `SP1` aggregation is working. The largest early
> control-plane gaps are now reduced. The remaining work is final production hardening: canonical
> task-state semantics, SP1 verification posture, hosted aggregate closure, endpoint strategy, and
> merged/live validation. Baseline estimate: about 1 week.
