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

Baseline estimate from the current state: **about 2 weeks of engineering time**.

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

## Why This Is Not Yet Production Ready

### 1. The current readiness model is too shallow

`GET /ready` currently checks only:

- L2 RPC `eth_chainId`
- queue availability

Code path:

- `bin/raiko2/src/server/ready.rs`

This is not enough for the current system shape. It does **not** tell an operator whether:

- the selected proving route is actually usable
- Boundless credentials and deployment settings are valid
- SP1 network proving is correctly configured
- external proving dependencies are reachable
- the currently configured `(network, l1_network)` pair is fit for end-to-end proving

Current consequence:

- control-plane readiness can report “ok” while proving is still operationally broken

### 2. Runtime and task status semantics are still not clean enough

The task lifecycle currently mixes:

- engine stage state
- runtime observer state
- route-specific runtime metadata

This area has been improved recently, including cancellation safety for shared engine tasks, but it
has not yet been fully hardened into a single production-safe status model.

Relevant code areas:

- `bin/raiko2/src/server/handlers/proof.rs`
- `bin/raiko2/src/server/state/runtime_observer.rs`
- `crates/runtime/src/lib.rs`

Production implication:

- operators can still observe misleading or incomplete task-state signals
- support/debug workflows are more expensive than they should be

### 3. Public docs, config defaults, and live behavior are still drifting

Recent behavior changes are real, but not fully reflected in the public operator docs.

Examples:

- `config.example.toml` now exposes Boundless batch quote controls:
  - `batch_quoted_mcycles = 1500`
  - `batch_quote_strategy = "raiko_agent"`
- `docs/operations.md` still states:
  - “Proposal requests quote `6000` mcycles”
- `docs/API.md` still shows historical SP1 examples such as:
  - `verify = true`
  - `cycle_limit = 1000000000000`

These are not harmless documentation nits. They directly affect:

- operator configuration
- live quote expectations
- production incident debugging

### 4. SP1 live success still depends on a reduced verification posture

Recent successful SP1 live runs used:

- `verify = false`
- `network_mode = reserved`
- `fulfillment_strategy = reserved`

Evidence:

- `target/compare/latest-proposal2670-sp1-network-request.json`

That proves the proposal proving path works, but it does **not** yet prove the final production
verification posture is fully locked.

### 5. Aggregate route behavior is not yet production-closed at the HTTP service surface

The underlying aggregation logic works, but the hosted route still needs final productization.

What is verified:

- `guest-launcher` can aggregate external proofs successfully
- live route registration for aggregate tasks exists

What is not yet proven to production standard:

- end-to-end `/v3/proof/aggregate` behavior for large real proof payloads
- body-size, validation, and operator ergonomics at the hosted route boundary

Evidence of route incompleteness:

- `target/compare/latest-proposals2669-2670-sp1-aggregate-from-tasks-status-4.json`
  still shows the aggregate task in `pending`

### 6. The service still depends on an external Taiko L2 endpoint for the working production path

Recent successful latest-proposal runs rely on:

- `http://34.121.5.35:8545`

The alternative “old-style local input-provider endpoint” route was investigated and is not
currently a usable production fallback.

Operational consequence:

- production readiness still depends on a single external L2 witness-capable endpoint being healthy
- endpoint strategy, fallback policy, and operational ownership are not yet fully documented

### 7. The current work is still sitting in a large unmerged diff

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

- control-plane readiness is weaker than proving capability
- task/runtime semantics still need production hardening
- public docs/config do not yet match live behavior
- aggregate route productization is incomplete
- SP1 verification posture and endpoint strategy still need final closure

In short:

- **old blocker:** “can the system prove?”
- **current blocker:** “can operators safely run and reason about the proving service?”

## Baseline ETA: 2 Weeks

This is a **single baseline estimate**, not an optimistic target.

### Work package A — Merge and normalize the current tree (2 to 3 days)

Required outcomes:

- split or otherwise make the current diff reviewable
- align docs/config/example files with real live behavior
- remove stale statements from `docs/API.md` and `docs/operations.md`
- ensure the readiness assessment itself does not drift from repo truth

Why it matters:

- without this, the repo has capability but no trustworthy operator-facing contract

### Work package B — Productionize runtime status and readiness semantics (2 to 3 days)

Required outcomes:

- define one authoritative task-state story for operators
- make `/v3/tasks` output consistent and predictable
- upgrade `/ready` so it reflects real route readiness, not just chain ID and queue connectivity

Why it matters:

- this is the difference between “works in successful samples” and “safe to operate”

### Work package C — Close the proving-route product gaps (2 to 3 days)

Required outcomes:

- lock the intended SP1 verification posture
- finish aggregate route production closure at the HTTP surface
- validate Boundless quote behavior and failure modes with the chosen policy

Why it matters:

- recent samples prove capability, but not complete route-hardening

### Work package D — Regression, runbook, and release validation (3 to 5 days)

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
   - `/v3/tasks` does not present contradictory states

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
> both `SP1` and `RISC0`, and external-proof `SP1` aggregation is working. The remaining work is
> production hardening: status semantics, readiness depth, route closure, docs/config alignment,
> and merged/live validation. Baseline estimate: about 2 weeks.
