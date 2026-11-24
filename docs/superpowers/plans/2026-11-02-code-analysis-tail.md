# Unreachable Bytecode Analysis Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Extend retained workload probes to code-analysis content branches and measure their literal node underpricing.

**Architecture:** A separate case producer composes the existing multi-account DELEGATECALL generator. Focused tests independently decode generated tails and execute the existing concrete loop oracle. The controller freezes qualified inputs before sequential guarded SP1 measurements, then records findings without changing prices.

**Tech Stack:** Python standard library, existing native/quote/SP1 executables.

**Spec:** `docs/superpowers/specs/2026-11-02-code-analysis-tail.md`

## Global Constraints

- Python uses the existing user-managed virtual environment; no new dependencies.
- Do not build a guest, change prices, limits, dependencies or host caps.
- Keep literal actual same-proposal SP1 PGU, each block node estimate and literal node sum, and complete proposal estimate distinct.
- Full charged block gas must be <=30,000,000 including the fixture Anchor; the existing 31,000,000 header is unchanged.
- Do not modify the prior 72-case campaign or its published fixture.
- No personal or machine-specific paths in portable source, docs or tests.

## Review Focus

- Unreachable PUSH data must not consume the suffix or create counted JUMPDEST bits: independently scan exact bytes.
- Salt placement and final opcode must separate content from padding: assert unique salts, aligned suffix and expected copy/padding states.
- The executed child stays identical: concrete execution must stop at RETURN byte 9 with the same context, routes and gas.
- Input IDs and output directories must not collide with prior campaigns or silently overwrite results.
- Model/source provenance and N0 controls must be explicit; measurements must use the literal block node denominator.

### Task 1: Reusable producer and regression coverage

**Files:**
- Create: `bin/zkgas/bench/cases/code_analysis_tail.py`
- Create: `bin/zkgas/bench/tests/test_code_analysis_tail.py`
- Modify: `bin/zkgas/bench/README.md`

**Interfaces:**
- Consumes: `multiaccount_code.make_case(..., child_call_kind='delegatecall')` and its `parent_code`, `gas_components`, `full_bound`, `CHILD_PREFIX`.
- Produces: `child_code(length, profile, index) -> bytes`, `make_case(length=4096, profile='stop', level='full') -> dict`, `prepare(output: Path, *, model_id=MODEL) -> int` and standalone CLI `--output-dir`, `--model-id`.

- [x] Write failing tests for the exact 24-case matrix, distinct identities, paired envelopes, byte lengths/patterns/salts, independently derived jump tables and padding, strict validation and overwrite rejection.
- [x] Run focused tests and observe the missing producer failure.
- [x] Implement the producer by composing existing parent/envelope logic, replacing only target runtime bytes and campaign metadata. Keep prior producer byte-exact. Catalog declares source hashes for all imported local producers, six profiles, 512 unique targets, two lengths and both levels.
- [x] Extend tests to execute representative full/zero loops through the existing concrete interpreter with an independent tail verifier, assert gas/routes/context, and reject wrong salt, tail pattern, executed prefix and final opcode mutants.
- [x] Add opt-in frozen-native smoke tests for all six profiles at both sizes (full/zero selected through a configurable artifact directory); verify success, terminal/route/gas counts and code dedup/omission. Do not invoke Cargo. Document catalog, native smoke and workload measurement composition with relative paths/environment-provided executables.
- [x] Run focused and existing multi-account tests, inspect complete diff/path hygiene, and commit scoped source/tests/docs.

### Task 2: Qualified campaign and findings

**Files:**
- Create: `docs/research/2026-11-02-code-analysis-tail.md`

**Interfaces:**
- Consumes: Task 1 catalog and unchanged frozen native/quote/SP1/model/ELF.
- Produces: immutable 24-input qualification, policy freeze, guarded actual observations and finite-domain result report.

- [ ] Independently verify all generated requests, native receipts and minimal authenticated witness/code coverage; quote and hash all canonical inputs. Preserve negative controls.
- [ ] Complete independent source review and freeze committed source, catalog, tools, inputs and model before actuals.
- [ ] Run all 24 sequential guarded SP1 execute-only measurements, retaining failed/null actuals with no implicit retries. Verify denominator joins, raw gas-to-PGU conversion, identical public outputs and owned shared-memory cleanup.
- [ ] Report literal node/proposal/actual, whole-block and N0-paired content contrasts, >2B severity and limitations. Keep pricing unchanged unless new evidence requires a separately reviewed fix.
- [ ] Independently review full source/results delta, run relevant lightweight checks, commit and update the existing draft tools PR. Preserve data locally for a new immutable Atlas revision; do not rewrite prior published data.
