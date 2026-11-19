# Node-observable estimates under proposal block-count growth

The prior [ordinary opcode study](2026-11-01-arithmetic-density.md) closes the observed
repeatable cheap-operation deficit with an empirical64-PGU normal-dispatch reserve, while
retaining absolute node underpricing. This followup tests whether the remainder grows with
block count. It introduces no price, limit, allocation or guest change.

## Frozen experiment

Use candidate `c8936124…`, native `b0497f45…`, quote `c10eae23…`, sampler `1998fb85…`,
SDK6.3 and unchanged source-bound ELFb76. The producer is
`0aabe961ca73faa002de756357608d07a307b57f`:
[usage](../../bin/zkgas/bench/README.md#proposal-size-controls-for-ordinary-opcode-composition).
The program manifest is declared source binding, not build attestation or production parity.
Actuals are execute-only normalized PGU (`ExecutionReport.gas *10//191`), not proofs or GPU time.

Two patterns, DUP16/POP and paired SWAP16, each use N0/N196608 per user at B1/B4/B16/B32.
The runtime bytes and user gas budget stay identical across block counts; each block includes
one user CALL plus Anchor. Each execution checks17 distinct seed words and a memory counter.
NUMBER/TIMESTAMP guard patterns are excluded because their expected context changes across blocks.
Sender nonces, headers, receipts and parent-state transitions are constructed normally, without
witness padding. Expected user opcode totals scale byB; inherited counter/guard metadata is per user.

The four B1 requests/canonical inputs/public outputs/inner quotes exactly match prior measured
validation inputs. Their original actuals are retained as **known references**. Only the twelve
new multi-block inputs receive new actual measurements. They are not fitted into the candidate.
All16 native inputs were independently qualified with concrete operand/stack/memory execution,
closed count/gas formulas, actual signed users/receipts and both quote rounding conventions.
Normal non-user count is12363 in the first block and12196 in later blocks in this constructor;
these observed fixture counts must not become assumed global constants.

## Three quantities and results

Every row compares actual identical-proposal workload with the complete proposal estimate and
node estimates for each block plus their literal sum. The aggregate-ceil node diagnostic remains
separate. No actual workload is allocated to individual blocks.

All12 new actual executions completed. Including the four known B1 references, full deficits
are0/16 and node deficits12/16. For the new twelve alone, full deficits are0/12 and node
deficits8/12. The remaining node deficits are every N0 input and the high-count B1/B4 inputs;
both high-count patterns are node-covered at B16 and B32.

| B32 case | Node sum PGU | Full proposal estimate PGU | Actual proposal SP1 PGU |
| --- | ---: | ---: | ---: |
| DUP16/POP,N0 |159,731,025|336,321,679|253,115,992|
| SWAP16 twice,N0 |159,731,025|336,321,679|253,116,579|
| DUP16/POP,N196608 per block |1,384,955,856|1,561,546,500|1,018,849,030|
| SWAP16 twice,N196608 per block |3,118,076,816|3,294,667,459|2,555,800,690|

All8 high-minus-own-N0 comparisons at equalB have node and full price increments covering
actual increments. All16 selected same-pattern/count block-growth comparisons (B1→4,4→16,
16→32 and1→32) are also covered by node and full increments. These are differences of proposal
workloads, not synthetic per-block actuals or a universal inequality.

For N0, signed actual-minus-node decreases from about153.44M at B1 to93.39M at B32.
For high-count DUP16/POP it changes from+139.01M to−366.11M; paired SWAP16 changes
from+132.67M to−562.28M. Thus this finite panel does **not** expose a deficit that amplifies
with either cheap-operation repetition or added blocks after the correction.

This supports the operation-growth correction and improves confidence for these patterns.
It does not identify the physical cost of an isolated opcode, prove the remainder is exclusively
startup, upper-bound all node workloads, or certify proposal admission/production deadlines.
The proving-only quoted gap itself varies with proposal input/MPT/payload composition.
Proposal-aware allocation to mining remains undecided. This study does not activate or tune
per-block/proposal limits; the separately deferred gas-saturation experiment remains deferred.

## Retention and remaining scope

Independent raw verification confirmed all48 new measurement-owned shared-memory objects
absent, with peak sampled PSS4,547,694,592bytes below6GiB. Audit SHA256:
`209b5cc2835022c01e21fdff017d02e6cb14921759b643689d7bcfcc918178b3`.

The reusable producer, focused positive/negative native tests, full Python framework checks and
usage remain in Raiko. Keep these12 new actuals and four known-reference identities separate
from the immutable84-input fitting/validation fixture. Raw reports, input hashes, all attempts,
resource receipts and independent audits are retained; no failed or rejected case receives PGU.

Next research should retain this three-quantity comparison while stressing composed execution
and state/input costs, rather than concluding node closure from full-quote coverage. A newly
observed scalable deficit would require its own source attribution, candidate and unfitted test.
