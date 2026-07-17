---
status: accepted
date: 2026-07-15
---

# Use an immutable proof artifact store

Published proofs use one environment-and-namespace-scoped runtime store as their authority:
production uses GCS, while local development and emergency operation may explicitly select the
ephemeral memory implementation. The store
contains the complete normalized Raiko `Proof`; provider request IDs are metadata only. Artifact
identity includes the explicit environment and namespace, concrete proof type, execution route, and normalized
request identity. Publication is conditional create and first-valid-wins because proof bytes are not
necessarily deterministic and replacing a completed task's artifact would break reproducibility.
Each proof payload is stored under its content hash, and a create-only manifest selects the
canonical payload for the logical proof reference.

Invalidation targets the selected manifest generation and content hash rather than banning content
globally. The manifest is removed with a generation precondition, so an identical later proof may
create a new generation without reactivating the invalidated publication. See
[Architecture](../architecture.md#proof-artifact-storage) for the complete object layout and
publication, cancellation, and recovery flows.

## Consequences

- A task becomes `Completed` only after its artifact is durable and registered as caller-readable.
- The same content at an existing key is an idempotent success; different content is recorded and
  discarded without overwriting the canonical artifact or regressing task state.
- Publication retries do not rerun proving. Recovery discovers a previously uploaded artifact by its
  deterministic key and completes registration before considering recomputation.
- Cancellation records a generation-scoped invalidation marker and conditionally removes only the
  selected manifest generation; immutable content may remain for retention and later deduplication.
- Each process selects one authoritative store backend. Automatic failover and dual-write are unsupported.
- Public and persisted artifact locations use the backend-neutral `proof_uri` field (`gs://` in
  production) instead of treating cloud object URIs as filesystem paths.
