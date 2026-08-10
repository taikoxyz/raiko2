# Client-Driven Runtime Recovery Design

## Problem

Raiko2 currently attaches every recoverable persisted root to the engine during startup. A root
that failed before creating a provider checkpoint can therefore reach the paid network prover after
a restart even when the client no longer needs that proof.

## Decision

Startup restores and validates persisted runtime state, but does not attach proof execution plans.
The next matching client request remains the authority that restarts work.

- Completed and cancelled roots remain terminal.
- Legacy route migration remains an explicit startup reconciliation action.
- Allocated, running, and failed roots remain persisted without engine children after startup.
- A duplicate client request checks whether the deterministic root has an active engine execution.
  If it does not, the request reattaches the canonical plan.
- Existing provider checkpoints remain part of the reattached plan, so a submitted request resumes
  its existing provider request ID. A root without a checkpoint creates a submission only after the
  client request arrives.

This is intentionally not a startup-cleanup mode. Deleting proof state would also delete resumable
provider checkpoints and could cause duplicate paid submissions.

## Safety Properties

1. Process startup alone cannot broadcast, stream, rebid, or otherwise create a paid provider
   submission.
2. Persisted provider checkpoints are preserved unchanged until a matching client request resumes
   the root.
3. A checkpointed recovery failure preserves the existing root instead of replacing it with an
   unsubmitted incarnation.
4. Concurrent duplicate client requests still use the runtime lifecycle CAS and attach at most one
   engine execution.
5. Canonical task IDs, request fingerprints, proof artifacts, and API response formats do not
   change.

## Verification

- Startup reconciliation does not attach an unsubmitted root.
- Startup reconciliation does not attach a root with a Boundless checkpoint.
- A matching duplicate request reattaches a stale root with provider progress.
- A failed reattachment preserves the checkpointed root and returns an error.
- An active root is not attached twice.
- Legacy SGXGETH migration behavior remains covered.
