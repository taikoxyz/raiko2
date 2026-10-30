# High-count minimal DELEGATECALL

## Question and scope

The frozen a298 model charges about 3,841.907 PGU for a confirmed DELEGATECALL,
much less than the conservative CALL and STATICCALL wrappers. Earlier finite
experiments emphasized memory expansion, returned bytes and nested rollback.
This checkpoint isolates high repetition of tiny child frames without large
memory, returned data or per-iteration creation costs.

The earlier retained CALL-memory and copying cohorts reached at most 512 loop
iterations, with additional dispatch/header or memory work. They do not replace
the direct 32,768-call density check. STOP and empty RETURN are the inexpensive
candidates. Empty REVERT and INVALID are intentional failure-path controls:
their existing terminal/failure pricing has a 5M PGU event floor, so the cheap
DELEGATECALL wrapper alone does not describe those complete quotes.

Keep literal actual proposal SP1 PGU / node-known zkGas as the ranking metric.
Complete proposal quotes and matched same-runtime repetition increments are
separate diagnostics. Startup-dominated controls do not establish scalable
underpricing. Measurement decides whether pricing needs to change.

## Finite construction

- One block, one user CALL, empty calldata, 15M transaction gas and the explicit
  evm-gas-only execution profile; ordinary validity checks remain active.
- A parent creates one tiny child before the loop, including in the N=0 control.
  Child runtime is STOP, RETURN(0,0), REVERT(0,0), or INVALID. The created address
  must be nonzero. No extra child account or code is added to prestate; a code
  byte string already used by a system account need not have a unique hash.
- Repeat DELEGATECALL N=0,1,1024,32768 with exactly 64 forwarded gas, zero input
  and output lengths, and fixed-width counter immediates. Check each return
  status against its expected success/failure and require empty return data.
- The parent always reaches STOP. Reverted/exceptional children are contained;
  exact completed opcode maps, user receipt, account nonce/runtime and spawn
  counts gate native qualification. INVALID itself is not a completed opcode.
- All 16 plans, controls, rejections and measurement failures remain visible.
  Do not remove a case merely because an old Unzen cap would have rejected it.

Frozen constructor b049, quote 0ead, sampler 1998, ELF b76, model a298 and their
existing source bindings are reused. No Rust/guest rebuild or tariff refit is
part of this measurement checkpoint. Terminal variants are distinct programs;
only within-terminal repetition changes provide matched scaling comparisons.

## Related source triage

Pinned WitnessDB uses indexed ancestor hashes and REVM uses a 256-slot block
hash cache. Warm account/slot accesses hit cached state; code is loaded once.
These facts reject a proposed repeated linear-history/MPT-materialization
mechanism, but do not validate their numerical tariffs. BLOCKHASH and warm-SLOAD
numerical checks remain follow-ups after this focused frame experiment.

## Measured result

All 16 canonical inputs completed actual SP1 execution under unchanged a298
pricing. There are 16 distinct inputs, 16 block occurrences and 32 included
Anchor/user transactions. No native exclusion or quote/SP1 failure occurred.
Complete quotes cover all 16 actual observations. All 20 matched repetition
increments, measured separately from N0 and N1 within each terminal series,
are covered by both node and complete price increments.

The high-count observations are:

| Child terminal; 32,768 calls | Charged user EVM gas | Actual proposal SP1 PGU | Node-known zkGas | Complete quote PGU |
| --- | ---: | ---: | ---: | ---: |
| STOP | 6,443,051 | 370,143,265 | 236,780,765 | 391,749,132 |
| RETURN(0,0) | 6,640,459 | 376,076,914 | 372,658,872 | 527,627,238 |
| REVERT(0,0) | 6,640,459 | 376,522,903 | 164,078,015,983 | 164,232,984,349 |
| INVALID | 8,540,203 | 370,284,512 | 164,076,358,639 | 164,231,327,006 |

STOP's N1-to-N32768 actual increment is 209,096,762 PGU, versus a node
increment of 227,089,701.8969489 PGU. This is about 6,381 actual PGU per
additional loop iteration versus 6,930 priced PGU. These increments include
the parent loop, status checks and child work; they are not an isolated
DELEGATECALL coefficient. Empty RETURN has 215,030,395 additional actual
PGU versus 362,959,658.4702302 priced PGU. The measured growth is covered
without changing tariffs, global discount, model identity or executable.

The literal actual/node maximum remains visible: STOP/N0 is 161,040,533 /
9,684,133 = 16.6293. That case still deploys one child, but runs no DELEGATECALL;
its complete quote is 164,652,500. STOP/N32768 has literal actual/node 1.5632
and complete/actual 1.0584. Proposal startup is separate from the node report;
do not replace either literal denominator with the complete quote or label
startup-dominated ratios as a demonstrated scalable deficit.

REVERT/INVALID are deliberately severe overquote controls, at approximately
436x/444x complete/actual for N32768 due to the existing 5M terminal/failure
floor. They are synthetic repeated failure transactions, not historical
Mainnet observations. Their pure EVM validity does not imply admission under
a future node zkGas cap. This checkpoint neither adjusts their conservative
prices nor changes the prior separate historical proposal comparison.

Independent qualification includes four additional two-block state probes and
five rejected negative probes; none has assigned actual SP1 PGU. The state
probes verify child runtime/nonce persistence even when delegated execution
reverts or halts exceptionally. Wrong status, short loops, parent OOG and
zero-forwarded-gas RETURN/REVERT fail qualification without canonical outputs.
Actual EVM gas independently satisfies RETURN=REVERT, INVALID−STOP=64*N and
RETURN−STOP=800+6*N. The maximum user gas leaves over 6.45M gas headroom,
excluding EIP-150 truncation as an explanation for the observed child status.

## Measurement identity and limits

Producer revision: `40a6e6aee508e11239767dd0842ec9effa602d9b`; its 494-file
source inventory is distinct from the original 484-file constructor, 490-file
quote and 67-file experimental program identities. Reused executable hashes:

- Constructor: `b0497f453b5b6f0e0210ed90a68f2072d63f7cb3337c781466428a8b77ef525b`.
- Quote: `0ead8567bab74b7277ed729010c9eb8b149dc50f822bcd039e105983f0100380`.
- Streaming SP1 sampler: `1998fb85e62fbbd190593ed1a71af044d197224174d45295f968da0d1a60330b`.
- Experimental ELF: `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`.

The SDK/executor is 6.3.0. Actual is the sampler's normalized `prover_gas`,
not raw `ExecutionReport.gas`, wall time, Unzen difficulty or GPU throughput.
Native, quote and SP1 reports bind the same exact canonical input and public
output. Peak sampled process-tree PSS was 3,091,400,704 bytes, about 2.88GiB,
below the existing 6GiB guard. No Rust or guest build was needed.

Independent raw-data verification checked all 16 canonical/native byte joins,
the original distinct source inventories, model/program identity, normalized
SDK PGU and public outputs, literal ranking and exact-scaled repetition deltas.
All 64 owned shared-memory objects from 16 receipts were verified absent in the
host namespace. The frozen audit SHA256 is
`3250e0dde5a0c02513f639191124d3165081595798d7f30c829d1b529a91150c`.

These are finite program-specific model-development observations, not a proof
of a global maximum, a gas-saturated block result or production guest artifact
qualification. No fresh Mainnet replay is claimed. The next numerical targets
are high-density BLOCKHASH and warmed storage reads; constant-time source
lookups alone do not establish that their prices are conservative.
