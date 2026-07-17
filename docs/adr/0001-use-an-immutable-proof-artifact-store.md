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

## Consequences

- A task becomes `Completed` only after its artifact is durable and registered as caller-readable.
- The same content at an existing key is an idempotent success; different content is recorded and
  discarded without overwriting the canonical artifact or regressing task state.
- Publication retries do not rerun proving. Recovery discovers a previously uploaded artifact by its
  deterministic key and completes registration before considering recomputation.
- Each process selects one authoritative store backend. Automatic failover and dual-write are unsupported.
- Public and persisted artifact locations use the backend-neutral `proof_uri` field (`gs://` in
  production) instead of treating cloud object URIs as filesystem paths.
