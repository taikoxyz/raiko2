# Account-journal comparison workload

The frozen `5f93909e…` model underprices a constructed, valid single-block account lookup
workload. With 11,166,706 block gas, its literal node quote is 3,519,436,414 PGU, while the
same proposal executes for 5,952,586,609 SP1 PGU. The 2,433,150,195 PGU deficit directly
meets the research priority rule: a block using at most 30M EVM gas can exhibit a positive
deficit greater than 2B PGU. This is a measured example, not an extrapolation or a maximum.

**Pricing is unchanged in this checkpoint.** The existing storage-journal reserve does not
price this separate account-table workload. **Follow-up triage defers the correction to TODO**:
the observed deficit requires a deliberately constructed combination and the largest measured
actual/node ratio is 1.691347x (actual/proposal 1.619960x). The absolute-deficit screen remains
true; it does not alone require an immediate pricing change under the updated project decision.

## Current triage: crafted-only observation, deferred

The demonstrated input combines a long common address prefix, a catalog selected for the
measured account map's bucket/tag, query placement after 511 other targets, and repeated warm
queries. The default dispersed control and concentrated short-prefix control do not underquote
at R512. No natural historical transaction exhibiting this deficit has been established by this
experiment; an online occurrence rate was not measured. The default control is itself synthetic,
so it must not be presented as a survey of ordinary transaction frequency.

Deliberate construction is cheap once this fixed profile is known: the long catalog search
tested 71,473,058 candidates in 655,486,207 ns (about 0.66s). This observation supports a
crafted-input classification, not an assumption that constructing such data is expensive or
that another proposal must retain the same profile. Keep the fixed inputs and the known deficit.

The project decision is to defer this sub-2x, deliberately constructed case and continue other
coverage. Revisit the correction if ordinary/historical inputs reproduce it or a measured
extension materially increases the ratio. No coefficient, runtime counter or price is changed
as part of this triage update.

Deferred prerequisite TODO: sample real Ethereum L1 blocks to examine whether account-set size,
shared address prefixes and repeated account queries naturally combine in this way. Use that
evidence when deciding whether to reopen the pricing TODO. This sampling is future work; the
seven synthetic cases do not establish L1 occurrence frequency.

## Frozen program and measurement

| Identity | Value |
| --- | --- |
| Model | `5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11` |
| SP1 ELF | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| Execute-only observer | `dfaee7ac41099df02e8a66d9b15b6c4b53d539d1a7e051a67b7180c64d6a0992` |
| Native constructor | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Current quote executable | `1390ee779fd09e03dacc4234624f01828ae93dc8995099f4ec8272f64ee69e0e` |
| SDK / executor / runner | `6.3.0` |
| Execution profile | `evm-gas-only` |

Actual PGU is the complete canonical GasVM result: `floor(raw ExecutionReport.gas * 10 / 191)`.
The report's normalized `prover_gas` is independently checked against this calculation.
The node denominator is literally `quote.node.sum_of_block_known_pgu`; the completed proposal
denominator is `quote.prove.total_pgu`. They remain separate. Raw Unzen difficulty values are
not PGU and are not used as another accuracy baseline in this experiment.

These are opened development cases using the specified historical ELF and qualified SDK.
Production SDK parity, a current-release ELF accuracy certificate and a universal workload
bound are not established. No guest or SDK was rebuilt, no proof was generated, and no
transaction was submitted to a network.

## Controlled inputs

Each constructed post-Unzen proposal has one block, its normal Anchor, and one user CALL.
The program first executes BALANCE on 512 different absent accounts, with the query address
last. It then executes one warm query plus 64 warm queries per loop repetition. It checks
calldata length, zero balances, the query, counters and memory sentinel before successful STOP.
No target code or storage is provisioned.

The query is `0x1122334400000000000000000000000055667788`. Each canonical Address20 contains
eight nonzero bytes; ABI calldata is 16,384 bytes, with 4,096 nonzero bytes. The three catalogs
have identical calldata gas, code at equal R, transaction envelope and EVM operation counts:

- Default: mixed suffixes, without the selected hash-bucket/tag concentration.
- Concentrated long prefix: the first 16 address bytes agree; the last four vary.
- Concentrated short prefix: the first four address bytes vary; the last 16 agree.

R0 and R512 are a matched control pair within each catalog. R1400 expands only the long-prefix
case after its R512 deficit is observed; it changes two loop immediates, preserving calldata
and the 14,997,601 transaction gas envelope. The old R0 is retained, without another execution.
This single-user fixture is not claimed to pack a full 30M-gas block.

A concrete byte interpreter independently checks opcode counts, address order, cold/warm
history, memory, success and gas. Native authentication checks the signed transactions,
receipts, initial/final state roots and 512 actual account-trie absence proofs using
`keccak256(exact 20-byte address)`. An independent verifier uses a separately implemented
RLP decoder, MPT walker and byte interpreter. Missing account code is not substituted for an
absence proof, and the old storage-slot absence gate is not reused.

## Results

All PGU columns refer to the same input within a row, including its Anchor and startup work.

| Catalog / R | Block EVM gas | Node PGU | Proposal PGU | Actual SP1 PGU | Actual minus node |
| --- | ---: | ---: | ---: | ---: | ---: |
| Default / 0 | 1,681,706 | 469,996,395 | 625,083,377 | 168,427,290 | -301,569,105 |
| Default / 512 | 5,150,506 | 1,585,220,173 | 1,740,307,156 | 212,868,458 | -1,372,351,715 |
| Long prefix / 0 | 1,681,706 | 469,996,395 | 625,086,705 | 185,502,489 | -284,493,906 |
| Long prefix / 512 | 5,150,506 | 1,585,220,173 | 1,740,310,484 | 2,294,789,732 | **709,569,559** |
| Short prefix / 0 | 1,681,706 | 469,996,139 | 625,087,985 | 174,173,981 | -295,822,158 |
| Short prefix / 512 | 5,150,506 | 1,585,219,917 | 1,740,311,764 | 839,317,576 | -745,902,341 |
| Long prefix / 1400 | 11,166,706 | 3,519,436,414 | 3,674,526,724 | **5,952,586,609** | **2,433,150,195** |

At R1400 the completed proposal quote also falls short, by **2,278,059,885 PGU**. A hypothetical
3.6B node-PGU limit would admit this block's quote, while its actual workload exceeds that
budget. This statement describes the example limit discussed for the research; it does not
claim the experimental model or that limit is deployed.

Small quote differences are accounted for. Long versus default adds 13 MPT path visits at
256 PGU each to the completed quote. Short versus default adds 19 visits, while its canonical
signed envelope is one byte shorter and costs 256 fewer payload PGU. Calldata byte cost is
unchanged; complete signed-envelope length is not forced to match through signature grinding.

## Why this is the account table

The fixed ELF's actual BALANCE path reaches
`Host::load_account_info_skip_cold_load -> JournalInner::load_account_optional ->
load_account_mut_optional -> HashMap<Address, Account>::rustc_entry`. Its 20-byte comparison
call is at `0x7835dc44`. The unrelated account `get_mut` specialization and generic Address
hash leaf are not silently treated as this primary path.

The Address20 hash is reconstructed from **each run's** primary-call registers:

```text
w0 = LE(address[0:8]); w1 = LE(address[8:16]); w2 = LE(address[16:20])
F(x,y) = low64(x*y) XOR high64(x*y)
H = F(F(own_seed XOR w0, shared0 XOR w1) XOR w2, shared0)
```

All seven own-input observations validate the address, caller, map and copied-key identities,
seed, shared word and computed hash. The relevant final mask is 1023. Both concentrated
catalogs realize all 512 target hashes at initial bucket 1015 and tag 97 in their own inputs;
the default has only the query itself matching that pair. This is observed realization,
not an assumption that another signed input retains the default's seed or layout.

The long and short R0 controls each execute 131,337 primary account comparisons. Their R512
actual PGU differs substantially despite matching EVM work and realized bucket/tag pressure.
This supports the extra cost of long equal byte prefixes; collision count alone cannot
explain the observed difference. Address comparison begins at network byte zero. Storage
U256's little-endian low-32-bit grouping must not be copied to Address20.

Register diagnostics stop after at most 400M guest instructions and retain at most 2,048
samples. The expensive probes have complete **canonical GasVM** measurements and partial
diagnostic instruction prefixes. Capturing every target cold key and a repeated query is
verified separately; truncated comparison counters are not whole-execution totals. In
particular, long R512 and R1400 can have identical prefix counters despite different complete
workloads. `samples_complete=false` is retained, including in completed R0 observations.

## Deferred pricing TODO and remaining scope

If this TODO is reopened, consider a simple, conservatively priced observation of account set
size, address-prefix groups and lookup-related work. Preserve the existing component prices
and check historical mainnet repricing before accepting a material change to the previous
proposal-level conclusions. A journal-container replacement is outside this proposed correction.

The actual Taiko wrapper finalizes ordinary transactions with `mem::take(state)`, so this
journal account map is transaction-scoped; the Anchor system marker can seed accounts before
the first transaction. Loaded account keys survive ordinary frame reversal, but failed
unmaterialized queries still need lookup accounting. Existing resource sets include declared
and synthetic addresses and therefore must not be described as exact physical table contents.

BALANCE is the demonstrated path. SELFBALANCE, EXTCODE operations, CALL/CREATE, storage's
implicit account lookup, rollback, SELFDESTRUCT, authorization and fee/system paths need
explicit observations or a documented conservative allowance. The DB's `CacheAccount` map
has a separate lifecycle and seed; this experiment does not close that surface. Capacity
transitions and mixed read/write/revert cases remain follow-up coverage.

## Retained tools and verification

The reusable producer, independent byte oracle and tests are under
[`bin/zkgas/bench`](../../bin/zkgas/bench/README.md#absent-account-journal-catalogs).
`hash_probe_catalog.rs --key-kind address20` generates the bounded external catalogs; omission
continues to use the original U256 interface. The original default storage fixture bytes and
hash CLI behavior are unchanged. See the linked README for commands and file contracts.

Focused checks run for this checkpoint:

- Managed-Python `test_*catalog*.py`: 21 tests, 20 passed and the explicit optional old Native3
  gate skipped. Seven separate new native inputs were actually constructed and authenticated.
- Standalone strict Rust helper unit tests: 3 passed; `rustfmt --check` and `git diff --check` passed.
- Independent complete tool-source review: passed, including 1,024 Address20 arithmetic vectors
  and 1,407 instruction words checked against the fixed ELF.
- Independent runner negative-path tests: 11 passed; seven native/quote and seven actual
  readbacks passed. This verifies the evidence and tools, not the old tariff's accuracy.

Each actual retained the 6GiB process-tree PSS, 600s, 192MiB output, 16MiB log and 576MiB
free-disk guards. The observed peak was 4.233GiB PSS; the longest run was 169.124s. All 28
distinct owned SDK shared-memory names were verified absent afterward. No sampling stage
was retried. A reporting-field typo after the first native control completed is preserved;
its continuation ran only the previously unlaunched probe.
