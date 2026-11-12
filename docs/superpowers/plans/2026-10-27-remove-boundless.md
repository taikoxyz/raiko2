# Remove Boundless Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Follow applicable AGENTS and independent review/verification requirements.

**Goal:** Completely remove the Boundless proving-market integration while retaining SP1, RISC0 local and TEE proving.

**Architecture:** Delete the Boundless adapter and route rather than introducing a disabled compatibility implementation. Preserve backend-independent request recovery, polling, queue leases, lifecycle fencing and persistence. Remove only helpers and dependencies that lose their final user.

**Tech Stack:** Rust workspace, RISC0/SP1 SDKs, Tokio, serde, TOML, offline Python experiment tooling.

**Spec:** `docs/superpowers/specs/2026-10-27-remove-boundless-design.md`

## Global Constraints

- Release is after Boundless shutdown. Old orders/databases are handled externally; no migration, cleanup, recovery compatibility or startup hook for them.
- Retain RISC0 local proposal/aggregate, SP1 local/network proposal/aggregate, native and SGX/SGX-geth routes and existing proof validation.
- Preserve SP1 request-ID persistence and same-request restart recovery, bounded retry, queue renewal, draining fences and GCS CAS semantics.
- Do not submit paid proofs, publish images, deploy, operate live services or change guest execution semantics.
- Preserve unrelated work. Use existing shared workspace Cargo targets as AGENTS directs; guest workflows retain their own cache controls. Python uses the existing user-managed virtual environment.
- Do not hand-edit generated ELF assets or lockfile checksums. Regenerate lockfiles through Cargo; inspect guest source/provenance requirements after manifest cleanup.

## Review Focus

- Remaining SP1 checkpoint persistence and cancellation behavior must survive deletion of Boundless-only progress variants/deadline logic.
- `risc0/local` must retain receipt/seal encoding and proposal/aggregate validation after the misleading contracts dependency alias is renamed.
- Obsolete route/config selection must fail ordinary validation rather than silently select another backend.
- Host-only, local-provers and engine-core feature combinations must remain coherent; no Boundless SDK or S3 feature is reachable from supported builds.
- Model-tool removal must not delete unrelated opcode/block calibration experiments or leave CI, docs, config, skills or commands referencing deleted active entrypoints.

## Task 1: Prover, pipeline and dependency removal

**Ownership:** `crates/prover/**`, `crates/pipeline/**`, `Cargo.toml`, all affected Cargo manifests/lockfiles. No server Rust or operational-doc edits.

**Interfaces retained:** `Prover`, `ProverProgressObserver`, `SubmissionCheckpointPermit`, generic pending-proof checkpoints, SP1 network progress and local RISC0 receipt codecs. `NetworkProverBackend` retains SP1; Boundless-only variants/types disappear. `PipelineKey` no longer contains `ShastaRisc0Network`. Call-graph inspection showed `PendingProofCheckpointIdentity` and the checkpoint-clear hooks had only a Boundless production caller; remove them rather than add unused SP1 clearing behavior.

- [x] Pin baseline source and inspect retained-path tests; add/update focused removed-route and retained RISC0/SP1 regressions where meaningful.
- [x] Remove Boundless modules/config/quote model/example and specialized exports/progress/resume/aggregation branches. Remove genuinely unused deadline outcomes/helpers while preserving non-cancelling retained persistence behavior.
- [x] Remove network RISC0 pipeline and route wiring; retain local RISC0 and SP1 behavior.
- [x] Remove Boundless dependency/features; rename the ordinary RISC0 contracts alias and preserve `encode_seal` with the same package/version. Coordinate bin/engine manifest feature changes with Task 2.
- [x] Regenerate affected lockfiles through tooling after all manifest edits settle. Report intended manifest/feature contract to other owners.
- [x] Run targeted pipeline/prover/RISC0 aggregation tests and supported feature checks using shared targets, then inspect the complete owned diff.

## Task 2: Server/runtime Boundless removal

**Ownership:** `bin/raiko2/src/**`, `crates/engine/src/**`. Coordinate manifest requests with Task 1 rather than editing manifests.

**Consumes:** Task 1's retained interfaces and removed Boundless/network-RISC0 variants.

- [x] Remove Boundless configuration fields, pair overrides, environment mapping, readiness/balance gates, engine setup/registration and route selection.
- [x] Remove Boundless-only task metadata, checkpoint conversions/comparisons, handler status/retry branches and tests. No old database compatibility or cleanup code.
- [x] Retain SP1/network status, checkpoint recovery, publication and GCS/runtime authority behavior. Keep common worker lease renewal/cancellation fences.
- [x] Replace useful backend-independent fixtures that happen to use Boundless with SP1/native fixtures; delete purely Boundless tests instead of retaining fake compatibility types.
- [x] Verify unknown Boundless config and removed `risc0/network` are rejected, while supported route/config loading and precedence are preserved.
- [x] Run focused server fixture/e2e, runtime and engine tests, including engine `--no-default-features`, and inspect the complete owned diff.

## Task 3: Tooling, operational surface and documentation cleanup

**Ownership:** `.github/**`, `docker/**`, `scripts/modeling/**`, Boundless-only quote fixtures, `.codex/skills/raiko2-risc0-zkgas-calibration/**`, `justfile`, `AGENTS.md`, `README.md`, `docs/**`, `config.example.toml`. No Rust or Cargo edits without root coordination.

- [x] Remove the production Boundless quote model generator/checker and exclusively dependent fixtures/calibration entrypoints. Preserve unrelated opcode/block experiments and measurement data.
- [x] Remove obsolete model/Boundless-S3 CI lanes and feature flags; retain remaining Rust and opcode calibration checks and update changed-path triggers.
- [x] Remove market credentials/offer/rebid/balance examples and update current route/build/operator documentation, agent entrypoints and skills consistently.
- [x] State the external old-state handling boundary concisely. Do not add drain/DB migration/cleanup procedures or a compatibility subsystem to the new application.
- [x] Run remaining affected Python tests through the existing venv, validate workflow/config syntax where tools are available, and inspect the complete owned diff for dangling entrypoints and path hygiene.

## Integration and verification

- [x] Root reconciles interfaces, runs a fresh active-source Boundless/dependency reference audit and checks retained runtime code for unrelated deletions.
- [x] Run workspace formatting/clippy and affected foundation/pipeline/runtime/prover/server/xtask CI test lanes; record any environment-only blocker separately.
- [x] Verify supported host/local-provers/SP1 network/RISC0 local feature combinations and absence of Boundless market SDK in supported dependency graphs.
- [x] Run relevant RISC0/SP1 guest build/provenance checks per AGENTS, using workflow-managed caches. Do not claim guest compatibility from host compilation alone.
- [x] Independent reviewer reads original request/spec and complete diff; independent tester checks retained behavior and failure cases. Address and reverify material findings.

After verification, inspect the complete base-to-head diff and validation, commit the scoped change,
push and open a reviewable PR requesting David. Do not merge or deploy without applicable user authorization.

## Validation record

- Workspace formatting, clippy with warnings denied, and diff checks passed.
- Foundation, provider, pipeline, preflight, queue, runtime and remote-poller test lanes passed. Final fixture-server suite passed all 437 tests.
- Independent behavioral verification passed prover tests (59), local RISC0 aggregation adapters (4), engine core (32), and SGX/SGX-geth mock/protocol/fixture integrations (16).
- Supported host, local-provers and standalone RISC0 feature checks passed. Guest build automation tests with `digests` passed all 37 tests.
- Both canonical guest builds and the combined provenance check passed. All 12 ELF/VK files remain byte-identical to the base; only generated provenance source fingerprints changed.
- Retained Python research suites passed: RISC0 23; opcode 43 plus one existing opt-in skip. Workflow/config syntax and remaining recipes were validated; local `actionlint` was unavailable.
- Independent review found and rechecked stale retained-route fixtures and an unintended SP1 serialized-key change. The established SP1 storage key is preserved; a literal existing-SP1-record regression and independent final metadata/observer/recovery rechecks passed.
- No paid proving, image publication, deployment, live GCS or real TEE execution was performed. SP1 coverage is helper/mock/checkpoint coverage, not a complete SDK gRPC submit/restart/fulfillment integration.

## Pre-flight coordination

| Tasks | Shared contract | Decision |
|---|---|---|
| 1 and 2 | Removed backend/route variants; retained progress observer and checkpoint types | Task 1 owns core types and all Cargo manifests. Task 2 adjusts consumers and requests feature edits. |
| 1 and 3 | Quote model include and generated artifact/fixture paths | Task 1 removes Rust includes and model artifact; Task 3 removes dedicated generators/fixtures/entrypoints. |
| 2 and 3 | Config/API examples and retained route names | Both follow the spec; Task 2 reports final config semantics for Task 3 documentation. |
| Task 1 | Local RISC0 retained while network removed | Preserve RISC0 receipt encoding and aggregation; rename only its misleading alias. |
| Task 2 | Full removal versus old persistent-state readers | No legacy Boundless compatibility or data handling; retain SP1 state behavior. |
| Task 3 | Delete quote tooling versus independent research | Delete only tools whose sole consumer is the removed Boundless quote model. |
