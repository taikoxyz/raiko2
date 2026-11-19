# Transaction order and repeated cold-frame workload

This panel retains36 new one-block SP1 execute-only observations under unchanged
modelc893 / SDK6.3 / ELFb76:24 shared-account transaction-order inputs and12
repeated-child-access inputs. Eighteen own-N0 controls match18 near30M charged-gas
full targets. All inputs and source were frozen before actual measurement.

## Construction and observation boundaries

The existing constructor gives all users the same calldata, target and gas
envelope. Order cases therefore use one shared role table and a real persistent
phase slot4096 to select SWAP16 density or dirty/dirty+hash caught-REVERT work.
Phase advances0→packing and remains nonzero at block end; journal slot2 returns
from0 through5/7/11 to0 within each transaction. Each transaction has its own
original-storage snapshot. The first phase write costs20000 gas and later clean
nonzero updates cost2900, plus the separately counted reads/guards. All phase,
calldata/dispatch and state work stays in both quotes and actual PGU.

Four or16 users run alternating, density-first or journal-first order, with half
executing each path. Own-N0 preserves the same role table, fixed body layout and
full-selected envelope. Intrinsic calldata gas, the nonbinding EIP-7623 floor
candidate, gross work, journal refund caps and every remaining header-gas prefix
were checked independently. This does not test arbitrary signed per-tx calldata
or multiple independent contract accounts.

The cold-frame panel repeatedly self-DELEGATECALLs from a four-site body. Each
child validates its input, reads absent slot2=0 and returns/reverts32 read-derived
bytes. Parent guards verify every status/length/value and the ending counter.
Cold RETURN retains newly warm storage; cold REVERT restores the warmth
checkpoint after every child; parent-prewarmed REVERT preserves the outer warm
entry. The parent does not read the target between calls. New transactions reset
EVM warmth even if a cached slot remains. No storage write/refund occurs here.

Coldness is not a count of DB rereads or MPT materializations. Raw quote
block-node resources, exact resources and block/parent MPT selectors remain
separate in the retained analysis; none is inferred by multiplying cold reads.

Thirty million means charged whole-block receipts including the167355-gas
fixture Anchor. The unchanged native header is31M; envelopes also account for
EIP-150 and journal SSTORE-sentry headroom. The explicit experimental
`evm-gas-only` profile bypasses the legacy Unzen cap. No new node limit was
selected, and30M-header admission is not demonstrated.

## Actual results

For the identical whole proposal, retain actual normalized PGU, each block node
quote and its literal sum, and the full proposal quote separately. These are
one-block proposals; actual PGU is not divided among transactions or opcodes.
Integer normalization is `ExecutionReport.gas * 10 // 191`. Neither the actual
nor raw Unzen difficulty is a GPU-time or proof-throughput observation.

| Full target | Charged block gas | Actual PGU | Node estimate | Full estimate | Node shortfall |
| --- | ---: | ---: | ---: | ---: | ---: |
| cold-return / 16 users | 29,973,515 | 1,072,679,887 | 1,961,025,081 | 2,115,967,168 | 0 |
| cold-return / 4 users | 29,989,935 | 1,083,619,319 | 1,978,066,498 | 2,133,008,585 | 0 |
| cold-revert / 16 users | 29,941,371 | 326,773,613 | 61,699,768,935 | 61,854,711,022 | 0 |
| cold-revert / 4 users | 29,968,715 | 327,007,202 | 62,339,835,209 | 62,494,777,295 | 0 |
| parent-prewarm-revert / 16 users | 29,975,515 | 1,071,124,076 | 376,296,425,425 | 376,451,367,512 | 0 |
| parent-prewarm-revert / 4 users | 29,984,151 | 1,078,615,250 | 379,989,208,801 | 380,144,150,888 | 0 |
| dirty / alternating / 16 users | 29,891,103 | 1,346,020,781 | 1,480,286,195 | 1,635,251,561 | 0 |
| dirty / densityfirst / 16 users | 29,891,103 | 1,336,435,036 | 1,480,286,451 | 1,635,251,817 | 0 |
| dirty / journalfirst / 16 users | 29,891,103 | 1,338,024,544 | 1,480,286,451 | 1,635,251,817 | 0 |
| dirty / alternating / 4 users | 29,958,015 | 1,375,761,170 | 1,466,464,507 | 1,621,429,874 | 0 |
| dirty / densityfirst / 4 users | 29,958,015 | 1,371,179,103 | 1,466,464,507 | 1,621,429,874 | 0 |
| dirty / journalfirst / 4 users | 29,958,015 | 1,375,281,052 | 1,466,464,507 | 1,621,429,874 | 0 |
| mixed / alternating / 16 users | 29,885,079 | 1,442,232,930 | 1,603,825,388 | 1,758,790,755 | 0 |
| mixed / densityfirst / 16 users | 29,885,079 | 1,434,280,883 | 1,603,825,388 | 1,758,790,755 | 0 |
| mixed / journalfirst / 16 users | 29,885,079 | 1,435,845,662 | 1,603,825,132 | 1,758,790,499 | 0 |
| mixed / alternating / 4 users | 29,964,169 | 1,467,746,470 | 1,592,232,945 | 1,747,198,312 | 0 |
| mixed / densityfirst / 4 users | 29,964,169 | 1,463,599,311 | 1,592,232,689 | 1,747,198,056 | 0 |
| mixed / journalfirst / 4 users | 29,964,169 | 1,466,746,721 | 1,592,232,945 | 1,747,198,312 | 0 |

Node underpricing remains18/36; full-proposal underpricing is0/36.
Maximum full-target node shortfall is0PGU;
maximum including controls is153,398,105PGU. Maximum actual
workload is1,467,746,470PGU. These maxima describe different metrics.
Strictly greater-than2B node shortfall within30M charged gas occurs in0 inputs.

Among18 exact own-N0 work increments, node covers18/18
and full pricing covers18/18. Scaled quote arithmetic is retained before rounding.
Increment coverage does not erase any absolute shortfall. Different full profiles
can fit different numbers of calls in the same gas budget; their total PGU
comparison is not an isolated per-call tariff.

## Deliberate REVERT overquote and the MPT distinction

Coverage of an observation is different from accuracy. The severe overquote tail
must remain visible: parent-prewarmed REVERT with4 users performs75,680 child
calls, costs1,078,615,250 actual PGU, and quotes379,989,208,801 node PGU, about352x.
The frozen accepted opcode:0xfd model explicitly charges5,000,000PGU per event;
75,680×5M=378.4B explains nearly the entire quote. Its feature-error charge is0:
this is the valid REVERT route's deliberate conservative tariff, not a newly
triggered feature error or missing-route fallback.

Cold REVERT with4 users performs12,416 calls, quotes62,339,835,209 node PGU and
costs327,007,202 actual PGU. The same5M reserve dominates. By comparison, cold
RETURN performs75,696 calls without a REVERT charge: node1,978,066,498 versus
actual1,083,619,319PGU, about1.825x. Thus the RETURN coverage result does not rely
on the terminal failure reserve. The order cases contain only2 or8 caught child
REVERTs per block and also keep their other work/cursor costs visible.

The [earlier delegatecall record](2026-10-30-delegatecall-repetition.md) already
retains this conservative failure-floor policy. These are constructed repetition
workloads, not new historical Mainnet observations. No coefficient changes here,
so the prior separate historical comparison is not rewritten. Under the user's
current priorities, retain this synthetic overquote as a low-priority precision
TODO; revisit it if historical applications demonstrate rejection risk. Admission
under a future node limit was not tested. These observations do not certify an
accurate per-REVERT price or every return-data size/value domain.

Across all six full cold-frame settings, the quote-observed block MPT counters
stay at32 path-node visits and12 materialization work units despite12,288..75,696
child calls. Those are retained host quote observations, not an independently
instrumented guest trace. Treating every cold SLOAD as a fresh DB/MPT decode would
misdescribe this evidence.

## Validation and interpretation

Source was frozen at `ada7f49adb60c7cc5fd260f0544c4ae6d1522924`.
Independent source review and all36 canonical native/quote qualifications passed.
The final complete stdlib discovery passed201 tests in50.808s:164 executed and37
optional skips. Separate native qualifications are not implied by those skips.
Retain14 independent native counterexamples, nine cold microcontrols and phase
insertion controls. Some counterexamples execute successfully as EVM transactions
but violate the intended experiment semantics; their rejection by the oracle is
not a claim that the protocol or tx pool rejects them. None receives invented PGU.

An intermediate test compared a large repeated write list through unittest diff
and could stall while formatting an expected failure. A bounded reproduction
identified the diff path; the frozen tests compare records individually. The
initial interrupted run is retained separately and is not reported as passing.
No guest, SDK, price or runtime policy changed to address this test issue.

Actual sampling completed36 bounded runs; peak sampled process-tree PSS was4,581,193,728bytes
under6GiB, and all144 distinct owned shared-memory objects were absent afterwards.
Raw attempts, artifact/source identities, controls, both deficit flags and resource
receipts remain retained. Measurements execute the guest; they are not generated
proofs, production-program certification or a deadline guarantee.

Apply the [gas-bounded priority and complexity rule](../zkgas-evaluation-standard.md#gas-bounded-priority-and-pricing-complexity)
to these finite whole-block observations. Low-priority gaps can remain TODOs;
no tariff correction is justified by this panel alone. Distinct accounts, arbitrary
signed calldata, adversarial trie topology and arbitrary workload mixtures remain
outside this panel. Earlier storage/rollback gaps and published fixtures are not
rewritten by these observations.

Maintained [producers, semantic tests and usage](../../bin/zkgas/bench/README.md#shared-account-transaction-order-composition)
stay in pending Raiko #296. Input/result data is retained in
[Atlas #26](https://github.com/taikoxyz/raiko2-atlas/pull/26) and its
[immutable data Release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-order-cold-frame-boundaries-v1).
Independent preseal and final archive checks passed:909 safe regular members,
compressed/decoded hashes and exact inner checksum closure. The unchanged
boundary-fixture workflow also passed using a fresh published GitHub download.
A temporary-index combined tree with pending Atlas #22–#25 passed all five
registrations/metadata checksum sets without changing real refs or index.
No automatic merge, deployment or pricing activation.
