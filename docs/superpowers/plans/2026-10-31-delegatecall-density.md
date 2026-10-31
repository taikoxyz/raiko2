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

- [x] Test first, then extend `delegatecall_repetition.py` with a density profile
  and focused tests in `test_delegatecall_density.py`; update bench README.
  Factor loop multiplicities without changing checked request bytes.
- [x] Independently review the complete delta, decode all eight programs and
  qualify native/quote behavior plus targeted negative/state controls. Commit
  the reviewed producer and freeze its source/catalog separately from binaries.
- [x] Execute each qualified canonical input once through frozen SP1. Audit
  original identities/public outputs, actual PGU, full closure, matched deltas
  and owned resources. Investigate any deficit without redefining the metric.
- [ ] Retain reusable tools/usage/results in Raiko and immutable data in Atlas,
  with independent archive verification. Keep source PR Draft/do-not-merge;
  no merge, deployment or unrequested guest build.

## Measured finding and correction follow-up

Producer f5502f38 / model a298 produced eight complete observations. Independent
raw audit found one full deficit and five underpriced STOP increments, with no
sampling failure. Preserve these parent observations. Freeze only confirmed-F4
at 6000 PGU using the exact 25% incremental-margin rule before new actuals.
Reviewed upstream correction is local commit 75737a81 / model 2fb4bb71.

- [x] Add six prospective PUSH0 cases at N0/4096/N65536; preserve the original
  24 requests. Root observed missing-API RED, then pure GREEN; independent source
  review and six parent-quote native checks passed. The old parent qualification
  catalog is distinct from the eventual candidate-model catalog.
- [ ] Build a comparison-only candidate quote from frozen CLI sources and an
  explicit reviewed Reth override, leaving committed dependency pins unchanged.
- [ ] Requote all prior 24 inputs, then sample six fresh actuals against the
  frozen candidate. Independently audit actual native repricing and holdouts.
- [ ] Requote the retained 50 Mainnet proposals to quantify historical impact;
  retain their original distinct SP1 program/SDK identities and actual PGUs.
- [ ] Publish the complete parent/candidate data and update guarded PR records
  after independent archive verification. No guest build or production switch.
