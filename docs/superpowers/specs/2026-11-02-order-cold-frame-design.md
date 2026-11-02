# Transaction order and cold-frame workload design

Continue the user's block-mining-aware zkGas investigation after the32-input
gas-budget panel. The objective remains to expose actual workload missed by the
literal node estimate, rather than to force estimates to equal actuals. More
than2B positive missing PGU within30M charged whole-block gas is high priority;
smaller observed discrepancies may stay TODO instead of adding pricing complexity.

## Chosen approach

Use the existing canonical ProgramRequest, native/quote tools and retained SDK6.3
ELFb76 sampler. Add two independently reviewable Python producers, meaningful
concrete semantic tests, and usage. Keep modelc893 frozen before actual sampling.
The user has authorized continuing these experiments autonomously; the remaining
matrix details are engineering choices, not new user-agreed protocol requirements.

Current ProgramTransactions repeats one calldata/target/gas declaration per user;
it does not accept an arbitrary user transaction list. For this panel use an
ordinary contract cursor and one shared calldata role table to select different
paths across signed transactions. Count all dispatch, cursor and state-root work.
Controls retain the same table, cursor, code layout and full-selected envelopes.
This deliberately tests a shared account. Arbitrary per-tx calldata and distinct
preloaded accounts remain outside this panel.

Alternatives considered: extending the host fixture API would support arbitrary
calldata but requires a new frozen native build; factory-created accounts add
setup/code-preparation variables. Neither is required to exercise the selected
order/rollback combinations. Reusing identical homogeneous transactions would
miss the new order axis. Do not change Rust, guest, SDK, prices or admission policy.

## Panel A:24 order/composition inputs

- Two combinations: paired-SWAP16 density plus dirty/caught-REVERT; paired-SWAP16
  plus dirty+hash/caught-REVERT. Half the users execute each path.
- Four or16 signed CALL users, with alternating, grouped density-first and
  grouped journal-first order; each has an exact own-N0/full pair.
- All users have identical shared calldata describing the order. The contract's
  persistent phase slot is distinct from journal slot2 and every memory counter.
  It advances once per successful user, with final phase equal to the user count.
  Each transaction has its own original-storage snapshot; later phase updates
  are clean nonzero writes, not first-transaction original-zero dirty writes.
- Bound table reads and validate outer/child input forms separately. Keep17
  distinct stack seeds and exact ending guards for density. For journal calls,
  validate status0, returndata length32, a work-derived marker and state rollback.
- Size each path's full work from charged gas, including intrinsic calldata,
  cursor operations, refunds, memory and all call/guard overhead. Select a common
  user envelope sufficient for the largest gross/EIP-150 requirement. Verify it
  fits every user's remaining native-header gas. Target at least99% of30M.

The persistent cursor is genuine contract work and remains part of actual PGU
and both quotes. Own-N0 differences are diagnostics; no cursor/startup cost is
subtracted from the literal node shortfall. Different order alone is not an
isolated causal estimate if other authenticated state/work also differs.

## Panel B:12 repeated child-access inputs

- Three profiles: cold-first child RETURN, cold-first child REVERT, and
  parent-prewarmed child REVERT. Four or16 users, each own-N0/full.
- Parent loops over a fixed unrolled body of self-DELEGATECALLs. Each child
  authenticates its input, SLOADs missing slot2=0, and returns/reverts exactly32
  bytes derived from that read. Parent checks status, length and value for every
  call, a final loop counter and a final read of slot2. No storage writes/refunds.
- The parent touches slot2 before the loop only in the prewarmed profile. In the
  cold-REVERT profile it must not read slot2 between child calls: that would warm
  it and defeat the intended repeated cold-access path.
- AtN>0, cold RETURN has one cold child read, subsequent warm child reads and a
  warm final parent read. Cold REVERT hasN cold child reads and a cold final read.
  Prewarm REVERT has one cold parent prewarm, then warm children/final read.
  AtN0 the final access remains cold unless prewarmed. Transaction boundaries
  reset EVM warmth even though caches and decoded witness nodes may persist.
- Compute child-call gas, memory and return-copy overhead exactly and qualify
  all charged receipts. Thirty-million charged gas includes Anchor; the unchanged
  header is31M. Keep the uniform full-selected envelope within remaining gas.

Warmness is not DB/MPT materialization. REVM can mark a cached slot cold on
rollback without removing its value. Report actual quote witness/path selectors
separately rather than inferring them from the2100-gas SLOAD branch.

## Qualification and measurement

New concrete tests independently model ordered calldata/stack/memory/storage,
per-transaction originals, checkpointed warmth and rollback. Bounded concrete
programs validate full fixed-body/count formulas; do not interpret millions of
events in routine Python CI. Independently qualify all36 final native inputs
and exact route maps/receipts before SP1. Preserve native-success semantic
mutations as well as rejects, with no invented PGU for either.

Retain small native controls showing cold RETURN versus REVERT's2000-gas
per-access difference and that an outer prewarm survives a reverted child.
The independent oracle must also handle transaction-boundary resets. Preserve
failed construction revisions explicitly; do not edit published fixtures.

Freeze committed source inventory, model/program/artifact identities and36
distinct canonical inputs before sequential actuals. Reuse existing6GiB PSS,
600-second and bounded-output guards. A censored actual is unknown, never low
priority. Preserve every attempt and owned-resource receipt.

For each identical proposal retain actual normalized SP1 PGU, each block's node
quote and literal sum, and the full proposal estimate. Reuse the maintained
gas-budget report without allocating proposal actuals to users/frames/opcodes.
Retain all18 controls, both absolute deficits and matched increments. These are
execute-only measurements, not generated proofs, GPU throughput or a bound.

## Deliverables and boundaries

Source/tests/usage and factual research notes stay in pending Raiko #296, Draft
and do-not-merge. Independent source review and independent qualification/raw
auditing are required. Immutable input/result data belongs in a new data-only
Atlas revision/PR after verification; no automatic merge or deployment.

Scope does not include arbitrary distinct accounts, crafted MPT key prefixes,
heterogeneous signed calldata, every workload family/order, an exhaustive
gas-only capacity search, proposal-shared budgets or retrospective tariff
simplification. A consequential gap may justify a separately reviewed correction;
no correction is assumed in advance. Low-priority attribution remains a TODO.
