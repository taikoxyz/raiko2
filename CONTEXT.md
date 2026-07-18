# Domain Context

## Glossary

- **Proposal proof**: A proof for one Taiko proposal. Public prover interfaces should use this term
  instead of Shasta-specific `batch` wording. The covered L2 blocks are contiguous; the HTTP prover
  v4 request carries an inclusive `l2_block_number_start`/`l2_block_number_end` range instead of
  the legacy `l2_block_numbers` list.
- **Aggregation proof**: A proof that aggregates multiple proposal proofs for submission.
- **Concrete proof type**: A caller-selected prover backend such as `risc0` or `sp1`. It excludes
  policy names such as `zk_any`.
- **Proof environment**: The deployment boundary in which proof work is submitted and consumed,
  such as `devnet`, `testnet`, or `mainnet`. It is identified by an explicit, required, immutable
  `environment` configuration value rather than inferred from `chain_id` or a deployment name.
  Proof tasks and artifacts from different environments are isolated even when every other request
  field is identical.
- **Runtime namespace**: The immutable single-instance persistence boundary inside one proof
  environment. Exactly one process uses it at a time, replacement processes never overlap, and
  namespaces never share data. It is distinct from the business environment and is part of task
  identity. The application intentionally has no distributed owner lease, owner epoch, or
  ownership heartbeat.
- **Global runtime fence**: The process-wide lifecycle gate that covers all task and external-store
  mutations for the namespace. Entering draining waits for in-flight writes and rejects new ones.
- **GCS generation**: A per-object version used for runtime-state CAS and exact artifact
  invalidation/deletion. It is not an instance epoch or a distributed lock.
- **Proof task identity**: The identity of proof work for one normalized request, concrete proof
  type, execution route, proof environment, runtime namespace, and effective prover configuration.
- **Proof artifact identity**: The identity of a published proof for one concrete proof type,
  execution route, proof environment, and proof request. Artifacts are never shared across concrete
  proof types, execution routes, or proof environments. Publication is create-only: an existing
  artifact with identical content is an idempotent success. The first valid publication wins; a
  later different artifact at the same identity is recorded as a publication conflict and discarded
  without overwriting the canonical artifact or regressing an already completed task.
- **Completed proof task**: A proof task whose final proof has been durably published and is
  available to callers. Successful proof computation alone does not make a task completed.
- **Proof publication**: The transition that makes a successfully computed proof durably available
  to callers. It is the completion boundary of a proof task, not a separate public task status;
  the task remains non-terminal until publication succeeds.
- **Runtime store**: The single authoritative backend for task state, remote checkpoints,
  publication state, manifests, and proof artifacts. A deployment selects GCS or ephemeral memory;
  dual-write and automatic failover are not supported.
- **Proof URI**: The backend-neutral location of a published proof artifact, exposed as `proof_uri`.
  GCS stores use `gs://`; memory mode uses `memory://`. `proof_path` is not part of the API.
- **Prover backlog**: Non-terminal prover work for one concrete proof type. `clean=true` means the
  selected proof type has no backlog; it does not mean every prover backend is clean.
- **Proposal fork**: The Taiko proposal rules active for a network and proposal, such as Shasta or a
  future hardfork. Clients should not select proposal forks in route names.
