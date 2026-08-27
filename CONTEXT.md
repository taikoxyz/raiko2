# Domain Context

## Glossary

Canonical definitions for environment, runtime namespace, the global runtime fence, GCS generation,
task and artifact identity, publication, runtime storage, and proof URIs live in
[CONCEPTS.md](CONCEPTS.md). This file records only request-domain terminology and contextual usage
rules.

- **Proposal proof**: A proof for one Taiko proposal. Public prover interfaces should use this term
  instead of the legacy fork-specific `batch` wording. The covered L2 blocks are contiguous; the HTTP prover
  v4 request carries an inclusive `l2_block_number_start`/`l2_block_number_end` range instead of
  the legacy `l2_block_numbers` list.
- **Aggregation proof**: A proof that aggregates multiple proposal proofs for submission.
- **Concrete proof type**: A caller-selected prover backend such as `risc0` or `sp1`. It excludes
  policy names such as `zk_any`.
- **Prover backlog**: Non-terminal prover work for one concrete proof type. `clean=true` means the
  selected proof type has no backlog; it does not mean every prover backend is clean.
- **Proposal fork**: The Taiko proposal rules active for a network and proposal. Unzen is the fork
  active on every live network; Shasta precedes it and still resolves for pre-Unzen proposals.
  Clients should not select proposal forks in route names.
