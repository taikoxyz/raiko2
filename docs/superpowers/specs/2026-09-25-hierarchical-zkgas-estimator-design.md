# Hierarchical SP1 zkGas Estimator Design

## Status

Superseded.

Replaced by
[`docs/plans/2026-09-26-zkgas-calibration-design.md`](../../plans/2026-09-26-zkgas-calibration-design.md).
The current task order and verified status live in the matching
[execution plan](../../plans/2026-09-26-zkgas-calibration-execution-plan.md) and
[progress ledger](../../plans/2026-09-26-zkgas-calibration-progress.md). This document is retained as
historical rationale and must not be treated as the current architecture source of truth.

This design supersedes the candidate-promotion parts of the June recalibration design and the
September relative-opcode design that assume the existing 102-key scalar reconstruction can become a
complete block candidate. Their provenance, controlled-measurement, proposal-barrier, immutable
artifact, bridge-isolation, and no-production-write rules remain in force. Existing accepted
relation, anchor, controlled-block, and structured-dynamic artifacts remain evidence for the
core-opcode component; they are not a complete candidate. The existing implementation plan must
therefore not be resumed unchanged: its candidate-construction and promotion tasks require a
replacement plan derived from this design.

## Problem

The controlled campaign has established a useful core-opcode relation model and accurate structured
models for six dynamic pure opcodes. It has also falsified two stronger assumptions:

1. one positive `raw EVM gas * multiplier` term is sufficient for every opcode; and
2. four synthetic anchors plus the exact rank-98 relation system necessarily reconstruct a positive
   absolute vector.

The structured dynamic diagnostic passes its frozen fit and holdout gates, while the legacy scalar
path reconstructs nonpositive lab-body values for `NOT (0x19)` and `JUMPDEST (0x5b)`. Those values
do not mean that either opcode has negative proving cost. They show that a contextual
matched-control system cannot be promoted as an exact additive absolute table without an explicit
common execution cost and nonnegative component model.

More importantly, the existing pure-opcode inventory is only one layer of proposal proving. A real
proposal also performs state and code access, copies, logs, calls and creates, precompiles, witness
and trie work, transaction processing, block validation and commitments, and proposal-level setup.
Validating a partial opcode table against total proposal `proverGas` would conflate missing work
with table error and allow higher-level offsets to hide lower-level omissions.

## Goals

1. Build a compositional SP1 cost model whose components have one explicit owner and sum to a total
   proposal estimate.
2. Calibrate and seal one scope at a time in this order: opcode, MKL/trie, transaction, block, then
   proposal.
3. Keep each layer free of later-scope semantics. In particular, an SSTORE model never depends on
   the eventual transaction outcome or final state root.
4. Prefer shared components and family models. Split by family or opcode only when frozen controlled
   holdouts show that the shared model is insufficient.
5. Use the smallest controlled fixture matrix that identifies the current layer. Scenarios exist to
   make coefficients identifiable, not to become production branching semantics.
6. Seal a complete review-only candidate before opening final Mainnet or Hoodi proposal results.
7. Preserve all current SP1 instruction-count bridge data as an independent, non-gating sidecar.

## Non-Goals

- Do not change the production Alethia schedule, runtime limit, or integer zkGas scale.
- Do not fit any coefficient, feature, split, threshold, or correction from proposal observations.
- Do not infer a component cost by assigning all higher-level residual to an opcode.
- Do not give each opcode an unrelated handwritten function when a shared typed model passes.
- Do not require exact component costs. A frozen approximation is acceptable when its controlled
  fit and holdout gates pass.
- Do not eagerly design RISC0 or cross-backend transport. SP1 is the measurement backend for this
  candidate.
- Do not treat all 256 byte values as distinct instructions. The opcode byte space has 256 slots;
  the current Unzen inventory names 150 instructions, and undefined values share INVALID semantics.

## Governing Rule: Scope Before Precision

The estimator is hierarchical:

```text
Z_total = Z_opcode + Z_mkl + Z_tx + Z_block + Z_proposal
```

Each component owns only work first introduced at that scope. A layer is calibrated from controlled
data after all lower layers are frozen. If a layer fails to converge or misses its holdout gate,
add features or fixtures at that layer. Do not borrow an offset from a later layer. Reopen a sealed
lower layer only when a reproducible lower-layer controlled case demonstrates that its model is
wrong.

For a controlled observation `i` at layer `L`:

```text
residual_L(i) = observed_proverGas(i) - sum(frozen lower-layer predictions(i))
```

Matched controls or a free intercept cancel unchanged higher-scope work. The fitter consumes only
the current layer's declared feature columns. A scenario name is descriptive and never changes
runtime pricing by itself.

## Layer Ownership

### Opcode Layer

The opcode layer owns the interpreter dispatch and the computation performed synchronously by an
executed opcode. Its canonical structure is:

```text
Z_opcode = sum(common_dispatch + opcode_model(opcode, event_features))
         + sum(shared_linear_memory_model(memory_event))
         + sum(precompile_model(precompile_event))
         + sum(spawn_wrapper_model(spawn_event))
```

Allowed opcode features are known when the opcode returns, such as raw EVM gas basis, exponent byte
length, copied words, input length, memory high-water mark, warm/cold access state, or an
original/current/new storage-value class. Forbidden opcode features include eventual transaction
success, eventual revert, final committed state, receipt content, block state root, or proposal
shape.

If a shared primitive is emitted separately, its work is excluded from the opcode body. For example,
MSTORE may be `mstore_base + f_mem`, but must not multiply total raw gas including memory expansion
and then add `f_mem` again.

SSTORE owns SSTORE handler execution only. Transaction rollback is transaction work. Witness/trie
lookup and final state-root work are MKL work. The same SSTORE model applies whether a later frame
or transaction succeeds or reverts.

More generally, a `StateAccess` opcode body owns the interpreter and state-semantics work needed to
perform that opcode. It excludes separately emitted witness hashing, cached-trie lookup, and trie
finalization events. This separation prevents a warm/cold opcode coefficient from charging the same
trie work again through the MKL model.

### MKL/Trie Layer

The MKL layer owns witness and state-trie work, even when a trie lookup is called from an opcode.
Current stateless execution has three distinct phases:

1. per-block witness initialization hashes supplied RLP nodes once, builds a digest map, and creates
   a `CachedTrie` anchored at the declared pre-state root;
2. execution-time account/storage reads perform cached-trie lookup and RLP decode, not a complete
   leaf-to-root rehash on every query; and
3. block finalization applies the final `HashedPostState`, updates affected storage/account paths,
   and computes the final state root once.

Witness initialization does not establish that every supplied leaf path is eagerly traversed. The
precise invariant is content-addressed nodes plus a root-anchored cached trie whose needed paths are
revealed and checked during construction or access.

The first MKL hypothesis is deliberately small:

```text
Z_mkl = f_witness_init(witness_node_count, witness_bytes)
      + f_state_finalize(changed_accounts, changed_slots, trie_update_features)
```

Execution-time lookup has an initial zero correction. Account and storage lookup counts remain
diagnostic. Add `f_lookup(unique_account_reads, unique_storage_reads, materialized_nodes)` only if
MKL-level holdouts show a repeatable residual. MKL initialization and finalization occur per
stateless block, not once for an entire multi-block proposal; a shared proposal node pool may reduce
input duplication without removing the per-block `SparseState` view.

### Transaction Layer

The transaction layer owns work whose lifetime is the transaction rather than one opcode:

- transaction decoding and validation;
- signer recovery and intrinsic processing;
- journal checkpoint, commit, and rollback;
- transaction gas accounting;
- receipt construction and transaction-outcome handling; and
- top-level transaction value transfer outside CALL/CREATE execution.

Executed work remains charged when a later revert occurs. Revert is not an SSTORE scenario or opcode
coefficient. A future transaction model may add a `reverted_transaction` or journal-size correction
if transaction-level holdouts require it. That refinement cannot change a frozen SSTORE model.

Protocol/system transactions use the same ownership rule: their transaction envelope belongs here,
and every opcode they execute belongs to the opcode layer. They do not receive a separate Anchor or
system-transaction proving-cost component. Internal CALL/CREATE value transfer remains part of the
corresponding opcode execution model; only its eventual trie commitment is MKL-owned.

### Block Layer

The block layer owns work that occurs once per block after lower-layer execution is accounted for:

- block environment and consensus validation;
- transaction-list and receipt-root commitments over transaction-layer outputs;
- block/header assembly and hashing;
- a residual block base for genuinely fixed work.

Receipt construction is transaction-owned; combining receipts into the block receipt root is
block-owned. The state-root calculation itself remains MKL-owned. An Anchor transaction is not a
special block-cost primitive: its envelope and opcodes follow the ordinary transaction/opcode rules.
A block offset may not absorb an unmapped opcode, missing MKL term, or missing transaction family.

### Proposal Layer

The proposal layer owns work performed once per proposal invocation:

- guest and runtime startup;
- proposal input decoding and top-level manifest checks;
- multi-block linkage and proposal-wide bookkeeping; and
- other work whose count is one per proposal, not one per block.

`proposal_startup` is added exactly once. Proposal results are validation-only and cannot fit or
repair this layer.

## Declarative Model Registry

Candidate artifacts store typed data, not serialized code pointers or 150 handwritten functions.
The evaluator has stable implementations for a small model-kind enum, and the registry maps each
active opcode or component to a model and its parameters.

Conceptually:

```text
ModelKind =
    StaticRawGas
  | Exp
  | Keccak
  | MemoryAccess
  | Copy
  | StateAccess
  | Log
  | CallWrapper
  | CreateWrapper
  | Halt
  | Invalid
```

The implementation may compile the registry to a 256-slot lookup array, but all undefined byte
values reference one INVALID model. They are not independently calibrated opcodes. Precompiles are
address-keyed components and never opcode slots.

Every executed event resolves to exactly one top-level model. Shared submodels are referenced by
content-addressed model ID and are charged exactly once.

## Core Opcode Models

The current Unzen inventory names 150 opcodes. The existing controlled manifest covers 102 pure
opcodes. Six of those require structured models:

```text
EXP (0x0a)
KECCAK256 (0x20)
MLOAD (0x51)
MSTORE (0x52)
MSTORE8 (0x53)
MCOPY (0x5e)
```

`CLZ (0x1e)` is a missing ordinary pure arithmetic opcode and joins the core inventory.

Ordinary static work uses a nonnegative opcode body plus a nonnegative common dispatch term. The
accepted relation system remains evidence, not an exact positivity guarantee. The new calibration
must fit a nonnegative approximation and report relation, anchor, fit, and holdout residuals. It may
not clamp individual negative reconstructions to zero or silently drop relations.

The sealed review-only core submodel has two explicit static exceptions. NOT (`0x19`) and JUMPDEST
(`0x5b`) are dispatch-only: their opcode-specific body coefficients are fixed to zero and are not
solver columns, but their events still pay `common_dispatch` exactly once. Relations containing
either key remain source evidence. When every referenced opcode has a final typed model, schema-3
replays the relation with its observed slope, prediction, absolute residual, and ordinary diagnostic
error. A static opcode supported only by dispatch-dependent relations is instead explicit
unsupported coverage; those relations retain their source identity and observed slope but do not
fabricate a numeric prediction or residual. Numerically replayed declared approximations do not enter
the nonzero MAPE/maximum-APE or exact-flat admission gates. Unknown, anchor, duplicate, or unobserved
dispatch-only keys invalidate the policy rather than silently widening it.

Structured dynamic models replace the static calculation for their opcode. They do not add a
correction to an already dynamic total-raw-gas multiplier.

The shared linear-memory model is:

```text
f_mem(old_words, new_words) =
    a * memory_growth_event
  + b * evm_memory_gas_delta
  + c * memory_4k_boundary_event
```

The current structured fit has fourteen polynomial/shared production-space parameters, plus one
declared non-fitted EXP bucket parameter:

```text
shared f_mem                                      3
EXP: constant, exponent_bytes, exponent_bytes^2  3
KECCAK: constant, zero-length, permutations      3
MLOAD/MSTORE/MSTORE8 constants                   3
MCOPY: constant, copy_words                      2
                                                   --
                                                   14
EXP short-exponent conservative bucket             1 declared approximation
```

KECCAK uses zero permutations for empty input and `floor(input_length / 136) + 1` otherwise, plus
the frozen zero-length term and shared `f_mem`. MCOPY adds copied words and `f_mem`. MLOAD, MSTORE,
and MSTORE8 have separate bases and share `f_mem`. EXP has no memory term. EXP byte lengths
`0,1,2,4` share a conservative body-space bucket equal to the maximum measured target body among
those rows. The quadratic is fit only from byte lengths `8,16,32` and retains byte length `24` as
its greater-than-four holdout. At evaluation, byte lengths through four use the bucket; larger byte
lengths use the maximum of the bucket and quadratic. Low-domain rows remain visible with APE,
overprediction, and underprediction evidence but do not enter polynomial error aggregates. Missing
byte length four, a changed low-domain set, or any measured low-domain underprediction fails closed.
All integer byte lengths `0..=32` must produce finite, nonnegative predictions under the exact same
piecewise evaluator used for artifact replay.

Physically additive scales and costs, such as common dispatch, raw-gas body scales, shared-memory
costs, and final per-event predictions, must be nonnegative. A contrast or branch-adjustment
coefficient may be signed when its basis is not an independently charged component. For example,
the KECCAK zero-length adjustment may subtract from its nonempty-path constant, but the resulting
empty-input event prediction must remain nonnegative.

## Remaining Opcode Families

The other named opcodes start with the smallest shared model that preserves explicit ownership:

| Family | Examples | Initial hypothesis |
| --- | --- | --- |
| Context | ADDRESS, CALLER, NUMBER, BASEFEE | dispatch plus raw-gas body |
| Context lookup | BLOCKHASH, BLOBHASH, CALLDATALOAD | raw-gas body; retain lookup diagnostics |
| Account/code | BALANCE, EXTCODESIZE, EXTCODEHASH | state-access family body |
| Copy | CALLDATACOPY, CODECOPY, other `*COPY` | non-memory raw gas plus shared `f_mem` |
| Storage | SLOAD, SSTORE | opcode/state-access body only |
| Transient storage | TLOAD, TSTORE | transient-state family body |
| Log | LOG0 through LOG4 | non-memory raw gas plus shared `f_mem` |
| Call | CALL, CALLCODE, DELEGATECALL, STATICCALL | wrapper plus shared `f_mem`; child separate |
| Create | CREATE, CREATE2 | wrapper/initcode work plus shared `f_mem`; child separate |
| Halt | STOP, RETURN, REVERT, INVALID | dispatch; output-range memory where applicable |
| Selfdestruct | SELFDESTRUCT | stateful opcode body; later system work separate |

When `f_mem` is charged separately, `non_memory_raw_gas` excludes EVM memory-expansion gas. CALL
forwarded gas and child execution never enter the wrapper raw-gas basis. A selected child is charged
once through its own events. INVALID does not multiply the transaction's remaining gas into proving
cost; it uses the executed failure-path model.

## Shared-First Promotion Rule

For each new family:

1. freeze every already accepted lower/shared component;
2. fit only the family base or raw-gas coefficient from a minimal full-rank controlled fixture set;
3. validate zero-growth, ordinary-growth, boundary, and out-of-fit points when memory is involved;
4. keep one shared family model when fit MAPE is at most 5% and holdout maximum APE is at most 10%;
   and
5. specialize only after a repeatable holdout failure.

Specialization order is:

```text
shared model
  -> shared model plus family correction
  -> family-specific model
  -> opcode-specific model
```

An operation whose isolated signal is below the frozen measurement floor may use dispatch-only or a
bounded approximation if its maximum controlled contribution is explicitly reported. Tiny signal
does not authorize a negative coefficient or a hidden higher-scope offset.

These approximations trade exact per-event attribution for a replayable conservative boundary.
Block-level validation must therefore measure their frequency-dependent residuals explicitly. A
fixed transaction, block, proposal, or other later-layer offset may not hide the accumulated NOT,
JUMPDEST, or short-EXP residual.

## Controlled Identification, Not Scenario Pricing

SP1 gas estimation yields one total `proverGas` observation, while the host-native trace supplies
component counts and semantic features. Component costs are therefore recovered from a controlled
design matrix:

```text
P_i = intercept_i + sum(feature_count(i, j) * coefficient_j)
```

Fixtures vary only enough to make this matrix full rank. For example, repeated writes to one slot
and writes to distinct slots can separate executed SSTORE count from unique-state effects. This does
not create success-specific or revert-specific SSTORE pricing. A fixture containing SSTORE followed
by REVERT is a transaction/MKL identification fixture, not another SSTORE model.

The measured SP1 guest remains uninstrumented. A separate host-native pass records the deterministic
event ledger and features from the same source revision. Candidate construction binds both artifacts
and rejects identity, ordering, or feature mismatches. No host trace value is treated as measured
`proverGas`.

## Artifact And Seal Structure

Each layer emits a content-addressed component with:

- layer and schema version;
- implementation, dependency, guest ELF, and launcher identity;
- frozen input artifact hashes;
- ordered feature and parameter declarations;
- coefficients in canonical Decimal form;
- exact fixture and observation hashes;
- fit, checkpoint, and holdout evidence;
- coverage and unsupported keys; and
- a status that distinguishes diagnostic evidence from candidate-eligible evidence.

Changing a lower layer invalidates every higher-layer digest. The full candidate contains at least:

```text
candidate.json
opcode-model.json
mkl-model.json
transaction-model.json
block-model.json
proposal-model.json
primary-observations.json
candidate.sha256
```

The SP1 instruction-count bridge remains outside this digest. Building or changing it must leave
`candidate.sha256` unchanged.

The current accepted opcode artifacts may be sealed as a `core_opcode_submodel` for provenance, but
that seal does not satisfy the full-candidate barrier and cannot open final proposal validation.
Schema-4 dynamic evidence and schema-3 core artifacts encode the declared approximation policy.
Schema-2 core artifacts are pre-P1 historical evidence, while schema-3 dynamic evidence and
schema-1 core artifacts are older formats; none of them is reinterpreted under the current policy.

When an implementation correction changes only post-processing of immutable controlled evidence,
it may be sealed as a separate `core_opcode_postprocess_derivation`, never by overwriting the
source calibration run or relabeling its identity. The derivation binds the historical calibration
identity/revision (which must resolve to a local commit), frozen manifest, formal decision ledger and
terminal raw rows, relation artifact, every validated anchor guest-input byte and its fixture/raw/fit
chain, block-row replay, and byte hashes for every consumed source file. It rehashes that exact path
set after replay. Schema-4 dynamic replay semantically validates the raw block rows and fits the
transfer parameters; it does not replay or claim a legacy block-calibration artifact, whose
positive-multiplier holdouts are incompatible with declared-zero controls. It also binds the clean
analysis revision and declared fitting basis that produced the new schema-4 and schema-3 outputs. A
derived directory is create-only, remains
`candidate_eligible=false`, and is not
a measurement run, a candidate component, or authorization to execute proposal validation. If the
source replay, dynamic support, or static rank check fails, it publishes no derivation directory
or partial artifact; it must never manufacture a core artifact.

## Completeness And Failure Rules

The full candidate may be sealed only when:

- every named active opcode maps to exactly one model;
- undefined opcode bytes map to the common INVALID model;
- every active precompile and declared spawn event has exactly one owner;
- memory expansion, child execution, state access, and system work are not double charged;
- all required coefficients are finite, every physically additive coefficient is nonnegative, and
  every in-domain event prediction is nonnegative;
- every layer passes its controlled fit and holdout gates;
- no proposal-purpose row participated in calibration or model selection; and
- candidate replay exactly reproduces every component digest and controlled prediction.

An unmapped active opcode, ambiguous ownership, missing feature, negative prediction, changed
source artifact, rank failure, or failed holdout blocks the seal. Do not substitute the current
Unzen table, a failsafe multiplier, or a later-scope residual to make a candidate complete.

## Final Validation

Only after the complete candidate and bridge sidecar are independently sealed may the frozen final
Mainnet/Hoodi corpus be opened. Proposal validation:

1. executes each immutable GuestInput with the bound SP1 estimator;
2. extracts the complete host-computable ledger;
3. applies the sealed layer models without refitting;
4. reports per-layer contribution, per-proposal APE, network MAPE, maximum APE, underprediction, and
   exact coverage; and
5. classifies the candidate without changing any coefficient, feature, ownership rule, or threshold.

The primary error remains:

```text
APE = abs(predicted_prover_gas - actual_prover_gas) / actual_prover_gas
```

Proposal output may identify the scope of a future experiment, but cannot repair the candidate that
was validated.

## Verification Strategy

Implementation must include focused tests for:

- opcode-byte lookup versus named active-opcode inventory and INVALID fallback;
- exact one-owner resolution for every event;
- common dispatch plus nonnegative opcode bodies;
- replacement, not double charging, of structured dynamic models;
- shared `f_mem` reuse and one memory-growth charge per event;
- CALL/CREATE wrapper versus child ownership;
- identical SSTORE pricing regardless of later transaction outcome;
- per-block witness initialization and final state-root ownership in MKL;
- layer digest invalidation when a lower component changes;
- refusal to seal partial coverage or failed holdouts;
- proposal barrier enforcement; and
- exact Decimal replay of the complete controlled prediction.

The implementation plan must add controlled fixtures incrementally by layer. It must not expand all
families and all system scopes in one unreviewable change. Each task ends with a sealed component or
an explicit controlled failure before work advances to the next scope.

## Execution Order

1. Adapt the accepted pure-opcode evidence into the nonnegative common-dispatch/core-opcode model.
2. Promote the existing fourteen-parameter dynamic diagnostic into the opcode model and add CLZ.
3. Add the remaining opcode families using shared-first controlled fits.
4. Seal the complete opcode layer.
5. Calibrate MKL witness initialization and finalization; add lookup only if required by holdouts.
6. Calibrate transaction work without changing opcode or MKL ownership.
7. Calibrate block work without absorbing lower-layer failures.
8. Calibrate the proposal-once component using controlled inputs only.
9. Seal the full candidate and independent bridge sidecar.
10. Run the frozen final proposal corpus once and report the result without refitting.
