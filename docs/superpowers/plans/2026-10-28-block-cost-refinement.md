# Block-aware cost refinement implementation plan

> **For agentic workers:** Use superpowers:subagent-driven-development for implementation and independent review/verification.

**Goal:** Cover measured block-local arithmetic and MCOPY deficits without materially changing historical mainnet proposal behavior.
**Architecture:** Alethia derives monotone prices from an immutable parent and sealed measurements; Raiko independently replays fixed inputs and checks historical deltas before pinning the reviewed upstream revision.
**Tech Stack:** Rust, generated authenticated JSON/Rust, exact-rational Python calibration and retained native/SP1 observations.
**Spec:** `docs/superpowers/specs/2026-10-28-block-cost-refinement-design.md`.

## Global constraints

- User authorized continued fixing, with discussion on material mainnet conclusion changes.
- Preserve MULMOD1600, payload ownership, default Unzen, global discount and all unrelated tariffs.
- No guest rebuild, new ELF/SDK claim, merge or deployment. Preserve unrelated changes and immutable fixtures.
- Mainnet discussion triggers: >5% median/weighted price uplift, >20% individual proposal uplift, or qualitatively changed distributions/normal-tx compatibility.
- One generator, no hand-edited generated artifacts; exact-rev #260/#284 stay Draft/do-not-merge.

## Review focus

- Counts/words must match executed operations; reject zero divisors and mismatched source evidence.
- Exclude reserved holdouts from fit; bind baseline model and all measurement identities.
- Preserve f_mem additivity and feature-error paths; a floor must never discount another term.
- Preserve startup, payload ownership, rounding and non-price output; do not force node totals to absorb shared startup.
- Keep historical score units, current model identity and actual ELF/SDK identity distinct.

## Tasks

- [x] **Evidence and baseline:** Independently derive event/word floors from current complete quotes and bound measurements; re-quote both90-proposal mainnet cohorts at model0a3 and seal exact observation identities. Save required-price maxima/exclusions.
- [x] **Upstream implementation:** In existing Alethia worktree, add immutable0a3 snapshot/compact evidence; write failing exact-rounding/scope/provenance tests, implement the deterministic successor and regenerate artifacts. Add runtime floor/memory/error regressions and pricing docs. Verify focused Python/Rust/no-std/Clippy as available. Do not push before historical gate.
- [x] **Replay gate:** Rebuild only affected native quote closure; re-quote retained boundary cases and180 mainnet proposals. Check selectors/public outputs/Unzen invariance, exact component deltas and ratio distributions. Pause for discussion if triggers fire. Seal then measure prospective holdouts with unchanged ELF/options.
- [ ] **Review and integration:** Independent reviewer inspects complete diff; independent tester checks behavior, fits, mainnet and holdouts. Fix/recheck material findings; update reviewed upstream#260 and exact-rev companion#284 with tests/docs. Read back heads/body/flags and actual CI; no auto-merge.

## Verified checkpoint

Candidate80459b4 covers all123fit+4zero-word controls and seven fresh prospective checks.
All298 retained inputs replay with price-only changes; 297 have measured PGU. Complete
underquotes fall56→3rows/two distinct dense EXP inputs, retained for follow-up. Two disjoint
90-proposal cohorts preserve roughly2x quotes; weighted uplift below1%, maximum proposal
uplift below1.46%, zero historical complete underquotes before/after. Independent source,
evidence and behavioral reviews passed. Upstream committed at1ef47962868b73cf45a81bd16f50f671493cda83.
