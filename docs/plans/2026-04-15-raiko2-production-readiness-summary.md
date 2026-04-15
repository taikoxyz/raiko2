# Raiko2 Production Readiness Summary

Date: 2026-04-15

## Current Status

`raiko2` is **not yet production ready**, but the core proving paths are now validated.

Verified capabilities:

- latest-proposal `SP1` live proof completed successfully
- latest-proposal `RISC0` Boundless live proof completed successfully
- `SP1` external-proof aggregation completed successfully
- latest-proposal preflight latency is now in a practical range

This means the current risk is no longer “can it prove?” The risk is “can we safely operate it as
the canonical production service?”

## Why It Is Not Ready Yet

The remaining blockers are narrower than they were earlier in the week. The largest early
control-plane gaps have been reduced, but the service is still not ready to be called the single
production path.

Remaining blockers:

- task status semantics are improved, but not yet fully canonical
- SP1 production posture is not fully closed yet
- hosted aggregate functionality is much closer, but not yet fully closed on the canonical live
  path
- the currently working path depends on a specific external Taiko L2 endpoint strategy
- the current state still lives in a large unmerged change set

Recent hardening already completed:

- `/ready` now checks configured L1/L2 RPC chain IDs, queue readiness, and default prover-route
  prerequisites
- `/v3/tasks` no longer mutates runtime state during reads
- public docs/config examples have been updated to match current behavior, including Boundless
  quote settings and `zk_any` sampling

## Baseline ETA

**About 1 week of engineering time**

This estimate assumes:

- current scope stays fixed
- current branch work still needs merge, regression, rollout, and live revalidation
- no new proving backends or major feature additions are added during the hardening phase

## What “Production Ready” Means Here

We should only call `raiko2` production ready when all of the following are true:

- live latest-proposal `SP1` and `RISC0` proving both succeed from the canonical deployed path
- live aggregation succeeds from the hosted route, not only from offline tooling
- `/ready` reflects real proving readiness
- `/v3/tasks` exposes stable and trustworthy state
- operator docs and config examples match real behavior
- the branch-local work is merged and revalidated after deployment

## Recommended External Statement

Use this wording for now:

> `raiko2` has validated the primary end-to-end proving paths and is in the final production
> hardening phase. The remaining work is final route closure, canonical task-state cleanup, SP1
> verification posture, endpoint strategy, and merged/live validation. Baseline estimate: about
> 1 week.
