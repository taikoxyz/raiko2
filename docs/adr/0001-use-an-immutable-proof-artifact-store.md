---
status: accepted
date: 2026-07-15
---

# Use an immutable proof artifact store

Published proofs use one environment-scoped `ProofArtifactStore` as their authority: production
uses GCS, while local development and tests may select the filesystem implementation. The store
contains the complete normalized Raiko `Proof`; provider request IDs are metadata only. Artifact
identity includes the explicit `environment_id`, concrete proof type, execution route, and normalized
request identity. Publication is conditional create and first-valid-wins because proof bytes are not
necessarily deterministic and replacing a completed task's artifact would break reproducibility.

## Consequences

- A task becomes `Completed` only after its artifact is durable and registered as caller-readable.
- The same content at an existing key is an idempotent success; different content is recorded and
  discarded without overwriting the canonical artifact or regressing task state.
- Publication retries do not rerun proving. Recovery discovers a previously uploaded artifact by its
  deterministic key and completes registration before considering recomputation.
- Each process selects one authoritative store backend. GCS/filesystem dual-write is unsupported.
- Public and persisted artifact locations use the backend-neutral `proof_uri` field (`file://` or
  `gs://`) instead of treating cloud object URIs as filesystem paths.
