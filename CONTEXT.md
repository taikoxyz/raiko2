# Domain Context

## Glossary

- **Proposal proof**: A proof for one Taiko proposal. Public prover interfaces should use this term
  instead of Shasta-specific `batch` wording. The covered L2 blocks are contiguous; the HTTP prover
  request carries the full `l2_block_numbers` list.
- **Aggregation proof**: A proof that aggregates multiple proposal proofs for submission.
- **Concrete proof type**: A caller-selected prover backend such as `risc0` or `sp1`. It excludes
  policy names such as `zk_any`.
- **Prover backlog**: Non-terminal prover work for one concrete proof type. `clean=true` means the
  selected proof type has no backlog; it does not mean every prover backend is clean.
- **Proposal fork**: The Taiko proposal rules active for a network and proposal, such as Shasta or a
  future hardfork. Clients should not select proposal forks in route names.
