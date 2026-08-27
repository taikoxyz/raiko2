# Domain Context

## Glossary

Canonical definitions for environment, runtime namespace, the global runtime fence, GCS generation,
task and artifact identity, publication, runtime storage, and proof URIs live in
[CONCEPTS.md](CONCEPTS.md). This file records only request-domain terminology and contextual usage
rules.

- **Proposal proof**: A proof for one Taiko proposal. Public prover interfaces should use this term
  instead of the v3 `batch` wording. The covered L2 blocks are contiguous; the HTTP prover
  v4 request carries an inclusive `l2_block_number_start`/`l2_block_number_end` range instead of
  the legacy `l2_block_numbers` list.
- **Aggregation proof**: A proof that aggregates multiple proposal proofs for submission.
- **Concrete proof type**: A caller-selected prover backend such as `risc0` or `sp1`. It excludes
  policy names such as `zk_any`.
- **Prover backlog**: Non-terminal prover work for one concrete proof type. `clean=true` means the
  selected proof type has no backlog; it does not mean every prover backend is clean.
- **Proposal fork**: The Taiko proposal rules active for a network and proposal, such as Shasta or
  Unzen. Every Taiko network is on Unzen as of 2026-08-06; Shasta remains a real fork that earlier
  proposals were proved under. Clients should not select proposal forks in route names.
- **Frozen identifier**: A `shasta`-spelled name that is part of a wire contract, an on-disk path,
  or a persisted value, and so cannot be renamed without breaking live consumers. These include HTTP
  routes such as `/prove/shasta`, request schemas such as `raiko2-shasta-request-v1`, `PipelineKey`
  variants such as `ShastaSp1`, proof-URI pipeline-key segments such as `shasta-sp1-local`, guest
  ELF and VK filenames, `xtask` profile names, and checked-in fixture and regression paths. The
  spelling records when the identifier was introduced; it does not select a fork. These names carry
  current Unzen work.
