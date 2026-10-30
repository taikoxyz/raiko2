# DELEGATECALL Density Implementation Plan

**Goal:** Measure eight minimally checked DELEGATECALL density probes.

**Architecture:** Extend the existing producer with an explicit density profile,
sharing construction and accounting with its unchanged default checked profile.
Reuse the frozen constructor, quote, sampler, program and bounded harness.

**Tech Stack:** Existing Python virtual environment; ProgramRequest/GuestInput.

**Spec:** [density experiment](../../research/2026-10-31-delegatecall-density.md).

## Constraints and review focus

- Preserve literal actual/node ranking and every control/rejection.
- Independent source review and independent behavioral verification under AGENTS.
- Default checked profile retains all 16 original request bytes; no duplicate
  constructor, host workaround, guest change or new pricing-model selector.
- Guardless completion requires exact child terminals and exceptional counts,
  not merely the successful parent receipt. N0 retains the same body/deployment.
- Existing memory, disk, time and owned-resource guards remain active.

## Tasks

- [ ] Test first, then extend `delegatecall_repetition.py` with a density profile
  and focused tests in `test_delegatecall_density.py`; update bench README.
  Factor loop multiplicities without changing checked request bytes.
- [ ] Independently review the complete delta, decode all eight programs and
  qualify native/quote behavior plus targeted negative/state controls. Commit
  the reviewed producer and freeze its source/catalog separately from binaries.
- [ ] Execute each qualified canonical input once through frozen SP1. Audit
  original identities/public outputs, actual PGU, full closure, matched deltas
  and owned resources. Investigate any deficit without redefining the metric.
- [ ] Retain reusable tools/usage/results in Raiko and immutable data in Atlas,
  with independent archive verification. Keep source PR Draft/do-not-merge;
  no merge, deployment or unrequested guest build.
