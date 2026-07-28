# Proof Identity Compatibility Design

## Goal

Prevent a host from reusing a proof produced by a different active guest or
remote SGX instance, without changing proof wire formats, task IDs, or runtime
storage schemas.

## Identity Sources

The host has one expected identity for each enabled proving lane.

- RISC0 derives the proposal and aggregation image IDs from the locally loaded
  guest ELF files.
- SP1 derives the proposal and aggregation verifying-key digests from the
  locally loaded verifying keys.
- Each remote SGX lane has an optional configured `(instance_id, address)`
  pair. When omitted, the lane starts unknown and learns the pair from the
  first successfully activated local proof.

ZK identities are reconstructed at each host startup. They are not operator
configuration and are not duplicated in runtime storage. Remote SGX learning
is process-local: operators who need a fixed identity across restarts configure
the pair explicitly.

## Rules

### New Proofs

- A configured or learned remote SGX identity must exactly match the proof
  header. A mismatch returns an error and never mutates the learned state.
- An unknown remote SGX lane accepts only a structurally valid candidate. It
  learns the pair after the proof has been activated as the root result. The
  pair is immutable for the process lifetime.
- ZK proof generation and verification continue to use their backend-native
  validation. In particular, a Boundless aggregation input remains a receipt
  plus carry data; it must not acquire a synthetic `uuid` requirement.

### Cached Artifacts

- A cached RISC0 or SP1 artifact is reusable only when its recorded program
  identity matches the current locally derived identity.
- A cached remote SGX artifact is reusable only when the lane has an expected
  pair and its proof header matches that pair. An unknown lane treats every
  persisted SGX artifact as a cache miss.
- A cache mismatch never exposes the artifact or its URI to an API caller.
  It is handled as stale completed work so ordinary proving can create a fresh
  result.

### Aggregation Inputs

- Completed cached child artifacts use the cached-artifact rule.
- Externally supplied inputs are new request data, not cache artifacts. RISC0
  network inputs retain receipt-based validation. Remote SGX inputs must have
  one consistent instance pair within an aggregate request, but do not teach
  an unknown lane.

## Lifecycle Boundary

For an unknown remote SGX lane, a per-lane finalization mutex spans canonical
artifact publication and root activation. The winner records its identity only
after root activation succeeds. A failure before activation leaves the lane
unknown. After activation, the identity remains fixed even if later cleanup
work retries; it represents the active remote lane rather than the retention
state of one artifact.

## Non-Goals

- No proof or public-input format change.
- No new durable expected-identity record.
- No replacement protocol for stale canonical artifacts.
- No change to on-chain proof verification authority.
