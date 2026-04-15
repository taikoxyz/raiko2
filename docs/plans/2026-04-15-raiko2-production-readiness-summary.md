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

The remaining blockers are operational and product-hardening blockers:

- readiness checks are too shallow and do not represent real proving-route health
- task status semantics are not yet clean enough for production operations
- docs/config/defaults still drift from live behavior
- SP1 production posture is not fully closed yet
- aggregate functionality is proven underneath, but not yet fully closed at the hosted API layer
- the currently working path depends on a specific external Taiko L2 endpoint strategy
- the current state still lives in a large unmerged change set

## Baseline ETA

**About 2 weeks of engineering time**

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
> hardening phase. The remaining work is operational readiness, state-model cleanup, route closure,
> and documentation/config alignment. Baseline estimate: about 2 weeks.
