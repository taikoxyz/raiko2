# Separating workload experiments from legacy admission

The new pricing model must be tested on EVM-valid workloads independently of the old
Unzen pricing limit. Exclusion by the legacy model is not evidence that the new model
would exclude the same transaction. The earlier 84 measured / 18 rejected return-copy
cohort therefore describes a legacy-admitted subset, not a complete new-model stress test.
Its original observations and exclusions remain valid historical evidence and are not
rewritten.

## Cause and explicit experiment

`ZkGasMode::Shadow` records the new pricing selectors while the production executor still
enforces the legacy 100M Unzen schedule. The fixture, full quote replay and production SP1
program all followed that path. Difficulty records usage; it is not a configurable budget.
Changing a host flag or supplied chain-spec map cannot relax the old program's built-in cap.

Read-only inspection of ELF
`416f5a475b17aa15914fb9387159d9eae9ecd474e7104da65ac22512ab56da2e` located its
`UNZEN_ZK_GAS_SCHEDULE` symbol at virtual address `0x7804b1e8`, file offset `0x4b1e8`.
The schedule contains a 100,000,000 block cap and 243,000 transaction intrinsic charge.
Both the native source path and that specific program enforce the old limit.

The offline tools now support one explicit execution profile throughout construction,
native quote and measurement:

| Profile | Behavior |
| --- | --- |
| `legacy-unzen` | Default production admission and existing report compatibility |
| `evm-gas-only` | Same Unzen accounting and execution rules with only the old admission cap raised to `u64::MAX` |

Checked arithmetic, EVM transaction/block gas limits, nonce checks, Anchor validation,
storage changes, rollback, witness/state-root validation and full proposal public-output
validation remain active. This experiment neither activates the new pricing model in a
node nor implements its admission policy. New-price budget selection belongs to the
consumer of the measured node-known quote.

The optional stateless `workload-lab` module supplies a single policy helper and plain
executor. The existing quote API and CLI default remain legacy. Explicit experimental
calls apply the policy to every block before any charge; a missing or already-used meter
is rejected. Default and explicit legacy artifacts were checked byte for byte.

## Program and report identities

The dedicated [workload program](../../guests/workload-sp1/README.md) uses existing public
proposal/stateless reconstruction hooks and unchanged canonical GuestInput I/O. It reuses
production crypto/syscall implementations and independently pins SP1 SDK/lib/primitives
6.3.0. Production guest sources, SDK pins, checked-in ELF files and release manifests are
not rebuilt or rewritten. The optional shared module does change source-fingerprint inputs;
guest-provenance CI must not be described as purely inherited drift after this addition.

The guarded harness freezes one profile and the selected ELF, manifest, model and tool
hashes. Experimental mode requires a program manifest matching profile/ELF/SDK. Each
attempt retains its manifest; native, outer/nested quote and SP1 reports must agree.
Configs without the new profile field preserve their prior frozen-contract shape.
An opaque program without a manifest is not relabelled as an experimental program.

The recorder binds source identities including the chain configuration embedded through
`include_str!`. Its manifest is a declaration, not an attestation that arbitrary supplied
ELF bytes implement the policy. The reviewed build and behavioral controls provide the
experimental evidence. Source hashes were captured and verified unchanged around the
actual build.

## Initial actual qualification

Experimental ELF:
`b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`.
Its 67 recorded source/configuration inputs remained unchanged through compilation.
The model remains
`537a965e51dd0416e3472a31cca8b067a85723fe4bbd08c52027ef3ca73f0e3b`; no tariff is changed.

An independently constructed three-block input uses about 193M legacy units per block.
The experimental program executes every transaction and matches native public output:
168,770,331 actual PGU versus a 222,642,088 complete quote. The unchanged old ELF exits
with guest code 1 on the same input; no PGU observation is assigned to that failed run.
The failure log does not identify an exact guest halt instruction.

The node aggregate for this three-block diagnostic is 66,748,361 when rounding the scaled
aggregate once, or 66,748,362 when summing individually rounded block prices. Both values
are retained; the one-PGU rounding distinction is not an operation-pricing gap. The new
return-copy campaign has one block per sample, so its literal node denominator is unaffected.
Clarifying the historical metric scope label without invalidating old frozen reports remains
a reporting follow-up.

An already-admitted empty-return control has the exact same canonical input in both
programs. It measures 161,162,789 PGU in the experimental ELF versus 158,430,766 in the old
ELF, a 2,732,023 program-baseline difference. Its unchanged complete quote is 161,246,448.
This leaves only 83,659 PGU of headroom in that control. Keep this baseline observation
separate from opcode or repetition increments; do not combine actuals from different ELFs.

Independent qualification verified raw normalization, inputs/public outputs, program and
profile identities and cleanup of all 12 owned resource objects across the two successful
runs and the old-program negative control. These initial checks establish the experiment
path, not coverage of the previously excluded return-copy cases.

The next measurement uses the exact same 102 ProgramRequests in a fresh campaign, including
the formerly excluded 18 and same-program controls. Retain EVM-gas/status failures if they
occur; only legacy pricing is removed as a selection boundary. Revisit older legacy-cap
exclusions separately rather than treating them as stress coverage.
