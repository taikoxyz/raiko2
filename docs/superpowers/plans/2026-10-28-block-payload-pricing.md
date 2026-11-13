# Block-local payload pricing implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development for coherent implementation and independent review/verification.

**Goal:** Make committed transaction payload visible in node shadow pricing without increasing the already complete proposal quote.
**Architecture:** Alethia owns the shared tariffs and transactional block collection; Raiko completes the same report with residual proposal inputs. Atlas keeps old inputs/observations immutable.
**Tech Stack:** Rust, generated authenticated pricing JSON/Rust, existing Python generator and frozen native/SP1 evidence.
**Spec:** `docs/superpowers/specs/2026-10-28-block-payload-pricing-design.md`.

## Global constraints

- Existing #260 is the maintained shadow branch; #284 is its authorized exact-rev companion.
- Default Unzen behavior and admission/difficulty semantics remain unchanged.
- One payload pricing implementation; no duplicated coefficients or compatibility side path.
- No guest/dependency rebuild solely to re-quote fixed observations; shared caches, bounded jobs, preserve unrelated edits and published fixtures.
- New production symbols have rustdoc; changed model ownership must remain authenticated and reproducibly generated.

## Review focus

- Included REVERT/HALT must retain payload, while rejected commits must not leak to the node prefix.
- Raw signed transaction size must not be inferred from calldata alone or unavailable in synthetic/RPC contexts without an explicit diagnostic.
- Attempted/candidate bytes must not be subtracted twice from committed bytes or hidden by saturating subtraction.
- Extra proposal candidates and integer rounding must preserve exact complete-scope accounting.
- Shadow changes must not change default Unzen, source pins or guest execution behavior.

## Tasks

- [x] **Upstream implementation:** Own `crates/evm/src/zk_gas/pricing/*`, EVM wrapper and `crates/block/src/executor.rs`, relevant tests and pricing docs. Write regression tests first, implement typed payload selectors and one shared evaluator, collect transactional selectors in the signed-tx pipeline, update known/complete breakdowns and authenticated ownership generation. Verify focused EVM/block/no-std lanes with available coherent caches. Record any unavailable required Cargo/clippy checks rather than claim success.
- [x] **Raiko integration:** Own `crates/zkgas-trace/src/{proposal_inputs,runtime_quote,reconstruct}.rs`, their tests, CLI projections/docs and exact dependency pins. Use upstream reviewed interfaces, reconcile committed payload with real block contents and retain additional-input charges once. Write mismatch/double-counting tests before the implementation.
- [x] **Independent engineering review:** Reviewer inspects the original request and whole cross-repo diff; tester independently checks rollback/admission boundaries, Unzen invariance, proposal residuals and evidence re-quotes. Investigate and fix all material findings, then have their authors re-check.
- [x] **Fixed-evidence evaluation:** Rebuild only a coherent cached native quote closure if necessary, keep old executables, re-quote frozen Atlas inputs under the new authenticated model. Bind input/model/executable identities and assert complete totals remain unchanged versus e529 for ownership-only changes; separately attribute prior MULMOD changes versus #284's older pin. Review historical node impacts without treating raw Unzen scores as PGU.
- [x] **Publication:** Inspect final diffs, commit scoped upstream changes and update #260, pin its reviewed exact rev in #284 and update that PR. Keep both Draft/do-not-merge, no activation/merge/deployment. Read back pushed heads and actions. Experimental evidence stays separate from published Atlas revisions.

## Verification checkpoint

All 257 native-valid saved inputs were re-quoted: complete fixed-point totals, public outputs
and Unzen scores stayed unchanged relative to e529. Node increments matched committed payload
prices. Independent review and behavioral verification passed. Reth workspace Clippy and
no-default-features checks passed; focused Raiko zkgas/trace Clippy and formatting passed.
Full Raiko workspace Clippy stopped at the free-disk guard and remains incomplete. Cached native
test closures passed; this is not a full Cargo test run or a new guest/SP1 certification.
Upstream #260 is published at `db54157ebd4de9fc4c1a2edc499b0db297d1bc15`.
Companion #284 payload implementation was published as `007fd8bc`; both PRs were read back
as OPEN, Draft and do-not-merge. The only reported upstream title check was skipped; downstream
CI results were not yet available at publication. No merge or deployment was performed.
