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

## Completed same-program campaign

The fresh campaign completed all 102 original ProgramRequests, including all 18 previously
excluded candidates. There are no native, quote, SP1 or integrity failures and no pending
samples. The maximum charged user gas is 14,744,273, within the unchanged 15M transaction
envelope. All 84 previously admitted canonical inputs, public outputs and node/full prices
match their original versions; their old-program actual PGU remains a separate observation.

| Full-copy workload | Actual SP1 PGU | Node-known PGU | Complete quote PGU | Charged user EVM gas |
| --- | ---: | ---: | ---: | ---: |
| 1 MiB RETURN, one call | 209,189,963 | 82,899,863 | 237,841,694 | 12,547,324 |
| 1 MiB RETURN, two calls | 210,085,558 | 91,087,146 | 246,028,977 | 14,744,259 |
| 64 KiB RETURN, 512 calls | 212,261,536 | 118,644,619 | 273,586,450 | 8,274,309 |

All 34 capacity triplets have equal complete native opcode maps and equal node/full prices.
Automatic copying still changes actual work: the largest full-minus-zero increment is
9,103,463 PGU for 512 caught-REVERT calls. All 33 same-program repetition increments are
covered by node increments; the largest incremental actual/node ratio is 0.442011799.

Complete quotes cover all 102 observations. The largest actual/full ratio is 0.999498627
(161,170,337 / 161,251,184), leaving 80,847 PGU in that small-return control. The literal
actual/node maximum is 25.562661300 for the empty-return control, dominated by proposal
startup. The opposite tail is also retained: the constructed zero-copy 512-REVERT case has
full/actual ratio 13.613619575. It is not a historical Mainnet regression.

Independent verification checked all request bytes against the original immutable archive,
all raw normalized PGU/input/output/profile/manifest/model/tool identities and cleanup of
408 owned resource objects. Maximum sampled process-tree PSS was 1,725,415,424 bytes.
No pricing coefficient was adjusted. This supports the current pricing on the measured
return-copy surface after removing the old selection bias; it is not an exhaustive bound.

The complete data is retained in [Atlas #11](https://github.com/taikoxyz/raiko2-atlas/pull/11)
and its immutable `fixture-unzen-return-copy-evm-gas-only-v1` Release. The actual remote asset
passed the unchanged Atlas integrity CI body: 1,742 archive files, including canonical inputs,
raw reports and explicitly labelled local-metadata projections. The reference ELF and host
executables are excluded; consumers record their own rebuilt program and repeat its controls.

## Next: workload at the EVM gas boundary

Retain the original `actual PGU / new node-known zkGas` objective and add a separate capacity
experiment: maximize actual SP1 workload subject only to the legal EVM gas envelope. First
verify actual chain transaction, block and proposal limits; the constructor's current
15M/31M/32-block bounds are experimental choices, not chain-limit evidence. Account for
refunds and distinguish charged receipt gas from execution gas when filling a budget.

Start with single-block saturation across precompiles, opcode combinations, memory, state/trie
and payload paths, then measure multi-block proposals with startup charged once. Use these
results to test the hypothesis that EVM gas alone could provide an acceptable proving-workload
bound. No conclusion about removing production zkGas limits follows before that work. Revisit
older legacy-cap exclusions separately rather than treating them as completed stress coverage.

### Current pinned protocol envelope

Source inspection of the pinned Alethia executor and Shasta derivation establishes:

| Boundary | Current rule | Enforcement |
| --- | --- | --- |
| Transaction | 16,777,216 gas including intrinsic cost | Osaka EIP-7825, executor configuration and REVM validation |
| Manifest block | 10M–45M, parent limit change at most 200 ppm | Shasta derivation validation |
| Derived header | Manifest limit plus 1M Anchor allowance; maximum 46M | Guest block construction |
| Normal source | At most 768 blocks | Unzen derivation constant |
| Forced sources | At most 10, each deriving one block | Inbox source count and forced derivation |

These imply a coarse topological ceiling of 778 blocks and 35.788B charged EVM gas per
proposal, before parent-limit and timestamp constraints. This is not a measured capacity or
an assertion that every such envelope is reachable from an arbitrary parent. Non-default
manifests require increasing timestamps no later than the proposal timestamp; fallback/default
handling differs, so a universal 360-block cap cannot be inferred from a six-minute interval.
The current tool still constructs one user transaction per block, with its own lower bounds.

A public Mainnet RPC observation at 2026-10-29T15:38:40Z verified chain ID 167000 and block
13,322,530 with header gas limit 46,000,000 (hash
`0x79b5c4cb2c9059a6d816afd8e6d14f88437e5094b4b310d7dfc351a58231c759`).
This confirms a real header setting, not a proposal-capacity measurement.

Block accounting uses receipt gas after refunds. London refunds can reduce charged gas by
up to 20%, so report charged gas separately from spent gas/refunds when testing refund paths.
User budget is header gas minus actual Anchor usage; the extra 1M allowance must not simply
be discarded or treated as a separate user-gas rule without checking executor enforcement.
Full proposal measurements also include startup once, Anchor, state/trie, payload and validation
work. Gas-per-transaction extrapolation alone cannot establish the full proving bound.
