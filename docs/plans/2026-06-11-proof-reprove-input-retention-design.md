# Proof Reprove And Input Retention Design

## Context

When a prover identity changes from `A` to `B`, old proposal proofs produced under `A` must not be
fed into an aggregate proof produced under `B`. Rejecting a mixed aggregate is necessary but not
sufficient: the system must also evict stale proof artifacts and reprove the affected proposals.

The current engine path executes a proposal as one composite task:

```text
preflight -> validation -> encode -> prove
```

Only the final proof is durable today. `GuestInput` and encoded input are intermediate scheduler
outputs, so deleting the proposal task forces the whole proposal pipeline to run again. That is
correct but too expensive. A prover identity change invalidates the proof receipt, not the proposal
input.

## Goals

- Evict stale proof artifacts when the current aggregate/prover identity changes.
- Keep reusable proposal input artifacts so stale proofs can be regenerated without repeating
  preflight and validation.
- Support proactive stale-proof cleanup when a remote prover registers or reports a new identity.
- Keep proof receipts as the final correctness source even if registration or active reporting is
  missing, stale, or malicious.
- Bound cache growth with an aggregate-aware and time-aware retention policy.

## Non-Goals

- Trust remote self-reporting as proof validity.
- Keep old proof artifacts around for cross-version aggregate fallback.
- Add a permanent cache for every historical proposal input.

## Artifact Model

Separate proposal execution artifacts into two layers:

| Artifact | Contents | Invalidated by prover identity change? | Retention |
| --- | --- | --- | --- |
| `proposal_input_artifact` | validated `GuestInput` and/or encoded input bytes plus metadata | No, unless request/input codec changes | aggregate reference + TTL |
| `proposal_proof_artifact` | proof receipt, public input, proof metadata, `proof_compatibility_id` | Yes | evict immediately when stale |

The first implementation should prefer storing encoded input bytes because the engine already has
`prove_encoded_with_observer(...)`. Metadata should include:

- `network_pair`
- `pipeline_key`
- `route`
- `proposal_task_ref`
- request fingerprint or serialized `ProposalTaskRequest` hash
- input codec/version hash
- prover config hash relevant to encoding
- created/updated timestamps

If an encoded input artifact is missing or metadata does not match the current request/codec, the
system falls back to the full proposal path.

## Reprove Flow

When a stale proof is found:

1. Delete or unregister only the stale `proposal_proof_artifact`.
2. Preserve the matching `proposal_input_artifact`.
3. Create a reprove marker for the proposal/task ref.
4. During planning, if the reprove marker exists:
   - use the cached encoded input if valid
   - enqueue a reprove-only task
   - write the new proof back to the canonical proposal proof ref
5. Aggregate depends on the reprove task and reads the canonical proof ref once it is regenerated.

The reprove task id should include either the expected current `proof_compatibility_id` or a
monotonic reprove epoch. This avoids scheduler reuse of an old succeeded proof task.

## Aggregate-Time Identity Change Handling

There are two aggregate-time failure modes:

- mixed input identities, such as `A+B`
- uniformly stale input identities, such as `A+A`, when the aggregate prover has already moved to
  expected identity `B`

The first case is detectable from the child proof receipts alone. The engine rejects aggregate
execution when child proofs do not share one `proof_compatibility_id`, and the runtime observer
prunes the non-target artifacts after a compatibility failure. When no explicit expected identity is
available, the target is chosen conservatively from pending/newer aggregate inputs.

The second case requires an external current-identity signal because `A+A` is internally
consistent. Current supported signals are:

- `AggregationTaskRequest.prover_config.expected_child_proof_compatibility_id`
- remote prover error payloads that include `expected_child_proof_compatibility_id`

When the expected identity is known, the engine rejects any child proof whose compatibility id does
not match it. If the mismatch is discovered by the remote prover instead, the gaiko2 client
preserves the expected id in the error string and the runtime observer prunes every aggregate input
artifact whose receipt identity does not match that expected id. This handles races where a remote
prover restarts or upgrades between proposal proving and aggregation.

Without one of these current-identity signals, `A+A` versus expected `B` is not knowable at the
raiko2 cache layer. Registration or active reporting can make that path proactive later, but it
should remain a cache-control hint rather than proof correctness evidence.

## Aggregate And Time Retention

Input artifacts should be retained by both aggregate liveness and wall-clock age:

- Mark input artifacts as referenced by an aggregate request while the aggregate is pending/running.
- Keep them after aggregate terminal state for a short grace window so failed aggregate/reprove
  recovery can reuse them.
- Delete input artifacts when no active aggregate references remain and `last_used_at + grace_ttl`
  has passed.
- Also enforce a hard `created_at + max_ttl` cap to prevent unbounded growth if metadata gets stuck.

Suggested starting values:

- `grace_ttl`: 1-6 hours
- `max_ttl`: 1-3 days

The exact values should be configurable. The cleanup should be conservative: deleting input
artifacts only causes a full proposal rerun, while deleting proof artifacts is required for
correctness.

## Prover Identity Registry

Registration can improve freshness but should not replace proof receipt checks.

### Pull Registration

Raiko2 can query a remote prover identity endpoint during startup and periodically:

```text
GET /identity
```

The response should include:

- route or backend kind
- service instance id
- current `proof_compatibility_id`
- raw identity material when useful for debugging, such as SP1 vkey hash, RISC0 image id, or TEE
  public key / instance address
- identity epoch or boot id
- started_at and expires_at

When the reported identity changes for a route, raiko2 proactively marks all proof artifacts with a
different `proof_compatibility_id` as stale and creates reprove markers. Input artifacts remain.

### Active Reporting

Remote provers can actively report identity changes to raiko2:

```text
POST /v3/prover/identity
```

This is useful after a remote restart because raiko2 can prune stale proofs before the next aggregate
request. Active reporting must be authenticated because it can trigger expensive reprove work. Use
one of:

- mTLS between raiko2 and remote prover
- operator-configured bearer token
- private-network ACL plus per-provider allowlist

Active reports should be treated as cache-control hints, not correctness evidence. A bad report can
cause unnecessary reprove work, but it must not make an invalid proof aggregate successfully.

### Registry Table

Persist the latest identity per route/provider:

```text
prover_identity_registry(
  network_pair,
  route,
  provider_key,
  proof_compatibility_id,
  identity_epoch,
  raw_identity_json,
  source,
  reported_at,
  expires_at
)
```

`provider_key` can be a configured remote prover name or URL hash. Local provers can populate the
same table from local ELF/vkey/image-id computation at startup.

## Correctness Rules

- Aggregation must still validate that every child proof has the same compatibility id.
- A registered identity may select which cached proofs are stale, but the proof receipt decides what
  identity a proof actually has.
- If registration is unavailable, planning still works in passive mode by comparing cached proof
  receipt identities and by pruning after aggregate compatibility failures.
- Reprove output must be checked against the expected identity when one is known. If a remote prover
  reports `B` but returns proof `A`, treat the proof as stale/invalid and do not aggregate it.

## Implementation Phases

1. Add durable proposal input artifacts and runtime metadata.
2. Add a reprove-only engine task that consumes an encoded input artifact and writes a fresh
   proposal proof artifact.
3. Change stale-proof cleanup to create reprove markers instead of removing the whole proposal task.
4. Add aggregate/time retention for input artifacts.
5. Add pull registration for remote prover identity.
6. Add authenticated active reporting for remote prover identity changes.

## Open Questions

- Whether to store validated `GuestInput`, encoded input, or both. Encoded input is cheaper for
  reprove, while validated `GuestInput` is more portable if encoding changes.
- Whether remote identity registration should be route-wide or provider-instance-specific when a
  route has multiple remote prover endpoints.
- How much identity material should be exposed in API responses versus only in logs/runtime DB.
