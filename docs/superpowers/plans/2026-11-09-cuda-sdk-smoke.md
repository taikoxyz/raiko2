# CUDA SDK Smoke Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans. Keep this manual test small.

**Goal:** Add a tiny, offline-input CUDA proving check through guest-launcher for SP1 and RISC0, and make its limited scope discoverable to agents.

**Architecture:** A dedicated `sdk-smoke` launcher stage uses existing SDKs with tiny fixed programs and checked expected journals. It produces and strictly verifies genuine proofs on local CUDA, with a CPU execute mode to validate fixtures without a GPU. It does not construct a Taiko proposal or run the production service.

**Tech Stack:** Rust, existing SP1 6.8.1 / RISC0 3.0.5 SDKs, optional CUDA, JSON fixtures.

**Spec:** User-authorized scope and design below.

## Scope and design

- This is an SDK/CUDA flow check only. It does not establish full proposal/aggregation correctness, production throughput, memory sizing, multi-GPU scheduling or service readiness.
- Reuse the committed SP1 opcode-lab ELF with a tiny arithmetic input. Use the exact-version upstream RISC0 loop ELF with its documented v1 compatibility wrapper and four-byte loop-count input. Record source, license, checksums and expected journals.
- Alternatives considered: a full valid Taiko GuestInput would add substantial unrelated work; new toy guest builds would add guest toolchain and ELF maintenance. Reusing existing small programs is the smallest useful test.
- `--stage sdk-smoke --proof-type sp1|risc0 --mode prove` must use CUDA, never the network, CPU proving or mock/development receipts. `--mode execute` is an explicitly labelled CPU fixture precheck.
- Real proof verification uses the matching ELF identity/key and compares the exact expected journal. RISC0 uses a succinct receipt and explicitly disables development mode. SP1 uses a compressed proof. Outer Plonk/Groth16 wrapping is outside this minimal check.
- No live RPC, proving-network request, key, deployment, GPU daemon installation or default GPU CI execution.
- Reuse the current prover SDK dependencies; no production guest ELF changes. Keep Python in the user-managed virtual environment.
- Use the shared workspace Cargo target for host checks. Do not change compiler profiles or remove shared target directories. No GPU/toolkit is available on the development host, so real CUDA execution must remain explicitly unverified here.

## Task 1: Minimal fixture and SDK launcher entry

**Files:** `bin/guest-launcher/Cargo.toml`, `bin/guest-launcher/src/main.rs`, a focused `sdk_smoke.rs` module, `tests/fixtures/cuda-sdk-smoke/`, and the workspace lockfile only if direct already-locked dependencies require it.

- [x] Add focused tests for accepted fixture parsing, exact expected journal checks, unsupported backends/options, and refusal to CPU-fallback when prove is requested without CUDA.
- [x] Add the optional CUDA feature, the dedicated stage and the smallest SDK execute/prove/verify paths for the two fixed cases.
- [x] Add tiny fixtures and upstream license/provenance without copying production ELFs.
- [x] Run launcher unit tests and both CPU fixture executions. Record actual outputs; no GPU-pass claim.

## Task 2: Packaging and discoverability

**Files:** `Dockerfile.cuda`, `docs/development.md`, repository `AGENTS.md`, and the fixture README.

- [x] Include guest-launcher and the tiny fixture data in the CUDA image; wire its CUDA branch into the existing image build.
- [x] Document exact local and container commands, GPU/toolkit prerequisites, expected results, and CPU-precheck versus real CUDA-proof status.
- [x] Add a short repository AGENTS routing pointer to this manual check. A new skill is unnecessary for two documented commands.

## Review focus and completion

- [x] Check prove mode cannot silently select network, mock or CPU proving, or accept fake RISC0 receipts.
- [x] Check CUDA feature forwarding, key/ELF/input binding and expected journal assertions.
- [x] Check fixture licenses/checksums, path portability, offline inputs and image paths.
- [ ] Run focused formatting, tests and lint; clearly distinguish unavailable CUDA compilation/execution from passing CPU checks.
- [x] Independent adversarial review and behavioral verification, then inspect the complete diff.

Validation status: 22 launcher tests and 28 independent ordinary CLI checks passed. Both tiny CPU programs produce their expected journals. Formatting and selective Docker context inclusion/exclusion checks passed; an initial Docker context defect was fixed and independently rechecked. Focused Clippy could not finish because the shared filesystem ran out of space. CUDA-feature compilation, a full CUDA image build and real GPU proofs remain unverified on this host. Keep the PR in draft until the missing checks can run.
