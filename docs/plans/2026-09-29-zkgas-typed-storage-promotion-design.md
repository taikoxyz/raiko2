# zkGas Typed Storage Promotion Design

Status: implementation-ready experimental design

## Purpose

Promote the sealed stateful SP1 calibration result
`experiments/opcode-gas/derivations/64065fa462311bdc1848e9d0` into the review-only
composite estimator. This closes the `SLOAD` and `SSTORE` execution-cost gaps without
claiming to price witness materialization, persistent state commit, trie hashing, or final-root
construction.

The promoted model remains experimental. It does not edit the production zkGas schedule or open
the frozen proposal-validation corpus.

## Source Of Truth

The trace must classify storage work from REVM execution state, not from interpreter gas:

- the current frame's `target_address` identifies the storage owner;
- the pre-step stack identifies the slot and, for `SSTORE`, the new value;
- the REVM journal's transaction id, access list, account cache, and storage slot identify
  warm/cold state plus original/current values;
- the post-step journal supplies a previously uncached slot's database-loaded original value and
  confirms whether the storage operation reached journal execution.

These fields are public on the concrete `TaikoEvmContext` journal used by raiko2, so no Alethia
change or parallel storage-state machine is required.

## Trace Schema

Operation trace schema 3 adds two typed opcode inputs:

```text
StorageLoad { access: warm | cold }
StorageStore { access: warm | cold, branch: noop | set | clear | reset |
               dirty_rewrite | restore_original }
```

`SSTORE` branch classification uses the execution-time tuple `(original, current, new)` and must
preserve the frozen campaign classifier's dirty-first ordering:

- `restore_original`: `original != current && new == original`;
- `dirty_rewrite`: `original != current && new != original && new != current`;
- an uncalibrated dirty no-op (`original != current && new == current`) is a feature error/gap;
- `noop`: `original == current && new == current`;
- `set`: `original == current == 0 && new != 0`;
- `clear`: `original == current != 0 && new == 0`;
- `reset`: `original == current != 0 && new != 0 && new != original`;
- dirty branches must be warm; a dirty+cold combination is a feature error/gap.

The typed input is emitted only for a normally completed EVM instruction. A malformed stack,
static-context rejection, stipend rejection, skipped cold load, any instruction-level EVM halt, or
missing post-step slot emits a typed feature error and therefore an estimator gap. This deliberately
fails closed for the rare case where `SSTORE` reaches the journal and then runs out of EVM gas: the
generic inspector cannot distinguish every pre/post-journal halt without duplicating REVM gas
accounting. A later frame/transaction revert still retains the already completed opcode event, and a
Taiko zkGas-limit rejection still retains it because the wrapped trace inspector runs before the
outer zkGas charge.

The trace records only the classified feature, not address, slot, or values. This is sufficient for
the model and avoids turning the estimator trace into a state dump.

## Promoted Model

The composite estimator consumes only the selected `M_typed` parameter vector from the exact sealed
result. It validates the result identity, file hashes, selected family, eligibility, ownership, and
all nine canonical Decimal parameters before use.

```text
SLOAD = sload_warm_body + is_cold * sload_cold_extra

SSTORE = sstore_branch[branch] + is_cold * sstore_cold_extra
```

The negative fitted cold modifiers are preserved verbatim. They are contextual SP1/REVM execution
contrasts, not protocol cold surcharges.

The estimator keeps the existing opcode registry immutable and adds a separate typed-storage model
source. Its execution-coverage rows for `0x54` and `0x55` become measured structured-storage rows;
all other rows retain their existing source and semantics.

## Ownership And Rollback

Storage opcode execution owns journal lookup/update and result-state construction measured by the
controlled fixtures. It does not own dirty-state persistence, witness validation, trie hashing, or
final-root work. Frame or transaction rollback changes state disposition but does not erase executed
opcode work from the trace. Higher layers may later refine state/trie cost independently.

## Acceptance

Before sealing a new composite estimator:

- Rust tests cover warm/cold `SLOAD`, all six `SSTORE` branches, access-list warming, dirty rewrite,
  restore-original, post-journal OOG, pre-journal failure, nested target address, schema rejection,
  and reverted-frame retention;
- Python tests cover sealed-source validation, exact Decimal prediction, coverage promotion,
  malformed typed inputs, feature-error gaps, and trace schema 3;
- focused Rust and Python suites pass;
- an independent reviewer inspects the full diff and an independent tester recomputes predictions.

Only after those gates pass may a new review-only estimator be sealed. The two existing ad-hoc
proposal diagnostics may then be replayed once without parameter tuning; they are diagnostics, not
the frozen final-validation corpus.
