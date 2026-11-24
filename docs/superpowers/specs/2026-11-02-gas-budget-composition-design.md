# Thirty-million-gas composition validation

The user authorized continued block-aware workload evaluation under the current
priority rule: more than2B missing PGU within30M EVM gas is high priority; smaller
discrepancies may remain TODOs. All pricing families and simpler future models
use this rule. This round obtains actual high-occupancy blocks rather than
extrapolating the prior storage/journal samples.

## Approach and scope

Reuse canonical ProgramRequest plus the existing frozen native, quote and SP1
tools. This is the selected implementation approach. Extending the Rust fixture
format or rebuilding a guest is unnecessary for the following families. Linear
projections remain diagnostics and cannot replace these measurements. Leave
minimal-proof storage growth/packing as a separate subsequent experiment: its
current storage_state interface does not accept explicit multiuser envelopes.

The root chooses32 new inputs, all with one controlled Unzen block, empty user
calldata, successful users and4 or16 user CALLs plus the ordinary Anchor:

- Density panel20: JUMPDEST, PUSH0/POP, DUP16/POP, paired SWAP16 and mixed
  JUMPDEST/DUP16/POP; each has ownN0/full at both packing counts.
- Journal panel12: dirty/direct, dirty/caught REVERT and dirty+hash/caught
  REVERT; each has ownN0/full at both packing counts.

The chosen full target is29,990,000 charged block gas, including Anchor. Actual
qualification must show29,700,000<=gas_used<=30,000,000 (at least99% occupancy).
The native constructor's31M header and15M per-user envelope bounds remain intact.
Select a sufficient per-user gas limit explicitly from gross execution cost;
user gas limits must still fit remaining native block gas at every transaction.
The selected full-case gas limit is identical in its ownN0 control. Thirty
million is the charged-gas research budget, not a claim that the same envelopes
would be admitted under a different30M header limit; retain the actual31M context.
Do not silently adjust limits, batches, packing or prices after an observed
failure. Retain failed plans. A later documented revision can correct a defect.

Each finite loop uses a fixed unrolled body, memory-resident counter and
explicit final counter/operand/state guards. Counts use fixed-width immediates.
Keep distinct seeded words for stack operations; loop state must not alias body
operands. Journal probes retain slot2's original value0/current nonzero dirty semantics,
parent cleanup, exact child status, returndata length and loop-derived marker.
Storage writes and REVERT work remain priced even when their effects disappear.
The journal panel prewarms its slot and self target; it does not claim cold-first
access coverage. Only canonical witnesses are built, without padding.

## Source ownership

Create two standalone generators and focused test modules:

- `bin/zkgas/bench/cases/gas_budget_density.py` and
  `bin/zkgas/bench/tests/test_gas_budget_density.py`.
- `bin/zkgas/bench/cases/gas_budget_journal.py` and
  `bin/zkgas/bench/tests/test_gas_budget_journal.py`.

Reuse existing assembler, typed request and storage/hash helpers as appropriate;
preserve all older producer/request bytes. Root integrates usage in the existing
bench README, runs sampling and records results. Tools stay in Raiko; Atlas gets
immutable data only. No new vendor format, decoder, pricing rule or public version
selector is introduced.

Root also adds `bin/zkgas/bench/gas_budget_report.py` and focused tests. This
read-only tool validates existing workload outputs and receipt-gas accounting,
then reports both shortfalls, saturation coverage and the strict2B priority.
It retains failed/unmeasured and multi-block records explicitly, without giving
them a fabricated single-block priority. An optional node limit is a separate
admission diagnostic; no legacy-Unzen limit is silently imposed.

## Evidence and verification

TDD includes matrix identities, deterministic/no-overwrite files, exact opcode
maps, loop/body semantic evidence, independent gas formulas (including memory
expansion/intrinsic/refund), strict domains and negative mutations. Mandatory
tests use only the standard library. Keep long full-gas loops out of an unbounded
unit-test workload; an independently decoded body/invariant and real native
completion/counts must still establish the intended high-count workload.

Independent source review and independent native/quote qualification precede
actuals. The tester must verify signed-user count/order, individual receipts,
all32 canonical inputs,99% full occupancy, semantic guards and failures. Record
modelc893, frozen artifact identities and declared source manifest before SP1.
Use unchanged SDK6.3/ELFb76; execute-only normalized PGU is not proof/GPU time.

Sample sequentially with existing6GiB PSS,600s,192MiB output,16MiB log and512MiB
free-disk guards. Never retry a censored input implicitly or invent its actual.
Retain complete proposal quote, each block's node quote and literal sum, actual
identical-proposal SP1 PGU, both absolute deficits/ratios, charged gas and2B
priority. Keep all16 ownN0 controls. Freeze coefficients throughout this panel;
any observed high-priority gap gets separate attribution and prospective fixing.
