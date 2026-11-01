# Dense ordinary arithmetic: proposal and mining estimates

## Scope and frozen evidence

The primary search remains actual proposal SP1 PGU divided by the sum of node-observable
block-mining estimates. Every case separately retains the complete proposal estimate, the node
estimate for each block and its sum, and actual proposal PGU. Underpricing of either estimate is
reported independently; full-quote closure does not establish node-admission closure. Matched
increments explain fixed versus repeatable gaps, without removing N0 or inventing block actuals.
Proposal-aware allocation back to mining remains a future design question.

The first cohort has24 inputs: ADD/MUL, four operand classes per operation, and bulk counts
0/4096/262144. Producer `f54230e583aa0bd3f6971811d204cbe9ebdfe72e` retains a fixed64-site body and
PUSH widths. It checks one semantic result, then repeats `DUP3; DUP3; op; POP` while preserving the
same independently decoded ordered operands. Target pairs execute N+1 times; counter ADDs execute
N/64 times and are counted separately. One user CALL, empty calldata, one block, 15M user gas and
the evm-gas-only profile are unchanged. Inputs use honest minimal construction, without witness
padding. The [producer usage](../../bin/zkgas/bench/README.md#arithmetic-density) documents the API.

Reused artifacts are native `b0497f45…`, quote `8624f630…`, model `bbce9be0…`, sampler `1998fb85…`,
SDK6.3.0, ELF `b76baf2e…` and manifest `8d5b4a48…`. This is execute-only normalized PGU evidence
under that frozen experimental program, not a generated proof, production-ELF parity or GPU
throughput measurement. The initial observations use the parent tariff. The candidate below changes only the normal
dispatch reserve; the actual-workload program, global discount and admission limits stay frozen.

## First24 actual observations

All24 native constructions and actual SP1 executions completed. **Node underpricing:24/24;
complete-proposal underpricing:8/24.** All eight complete deficits occur at262144 bulk operations.
The worst complete actual/estimate is `226685516 / 196098654`, about1.15598x. The literal worst
actual/node remains the retained ADD-zero N0 control: `161002032 / 6724859`, about23.9413x.
Its large startup-related gap is not hidden by matched increments or by full-quote coverage.

| Bulk operation / operand class | User EVM gas | Node estimate PGU | Proposal estimate PGU | Actual proposal SP1 PGU |
| --- | ---: | ---: | ---: | ---: |
| ADD / zero | 3,068,494 | 41,156,824 | 196,098,654 | 226,685,447 |
| ADD / small | 3,068,494 | 41,156,824 | 196,098,654 | 226,685,462 |
| ADD / full carry | 3,068,494 | 41,156,824 | 196,098,654 | 226,685,516 |
| ADD / mixed256 | 3,068,494 | 41,156,824 | 196,098,654 | 226,685,476 |
| MUL / zero | 3,592,784 | 51,254,972 | 206,196,802 | 235,601,170 |
| MUL / small | 3,592,784 | 51,254,972 | 206,196,802 | 235,601,251 |
| MUL / overflowing wide | 3,592,784 | 51,254,972 | 206,196,802 | 235,601,197 |
| MUL / mixed256 | 3,592,784 | 51,254,972 | 206,196,802 | 235,601,221 |

The table selects high-count rows; all N0 and4096 controls remain in the retained24-case scorecard
and both underpricing counts. All24 same-operation/class N0/N4096 matched increments are also
undercovered by node and complete estimates. Thus fixed startup alone does not explain the gap.

Operand values change high-count actual PGU by fewer than100PGU in either family. Within this
finite matrix, large operands are not the main source. These observations price a composite
stack/arithmetic/loop workload, not isolated ADD or MUL. Do not raise either arithmetic coefficient
solely on this attribution. The paired controls below locate a shared stack/loop deficit before
any tariff decision.

## Twelve paired duplicate/discard controls

Producer `5e1a2b239c45e3f6694fe4b3860e7ee0fc3b564b` adds a separate `dup-pop` profile. It changes
only the 64 bulk arithmetic sites to POP, yielding `DUP3; DUP3; POP; POP`; the arithmetic warmup,
operands and loop remain. ADD/MUL zero and mixed256 references each use 0/4096/262144 bulk pairs.
The original 24 requests and case metadata remain byte-identical. New control N0 inputs are
distinct because their unused code differs; each control retains its own N0 and original-request
hash. The default catalog's generator hash updates truthfully with the new producer source.

All 12 actual measurements completed: **node underpricing 12/12, complete-proposal underpricing
4/12**. For 262144 bulk pairs, ADD-zero control estimates 34,418,543 node PGU and 189,360,374
proposal PGU against 220,737,060 actual PGU. The other three high-count controls have actuals in
220,737,096..220,737,251 PGU, with correspondingly retained node/proposal estimates.

To explain the shared deficit, compute differences of matched increments:

```text
actual_relative = (arithmetic_actual_N - arithmetic_actual_ref)
                - (control_actual_N - control_actual_ref)
node_relative   = (arithmetic_node_N - arithmetic_node_ref)
                - (control_node_N - control_node_ref)
```

All 12 N0/N4096 reference comparisons have `node_relative >= actual_relative`. For zero operands
and N0 to262144, ADD's relative actual increment is 5,948,418 PGU versus a 6,738,281 node increment;
MUL's is 14,863,959 versus 16,836,391 PGU. Thus the arithmetic-versus-POP difference is covered in
this matrix, while the duplicated/discarded stack-and-loop work remains underpriced. This is a
composite difference, not an identifiable isolated arithmetic, DUP or POP cost. It changes neither
primary full-ratio judgment: the combined 36 cases retain 36 node deficits and 12 complete deficits.

Independent raw verification recomputed all joins, normalization, comparisons and resource cleanup.
All 48 control-owned shared-memory objects were absent in the host namespace. Its report SHA256 is
`933615e2941a078a853822acb241f7e8298178ee975d75aad5c3ad045c52ad64`.

## What the original calibration did

The inherited registry `f3fbef69…` uses the core relation
`body_scale * lab_body + common_dispatch`, with body scale about1.1326743533 and common dispatch
about12.9748846915 PGU. Transfer anchors include POP, PUSH0, DUP1 and SWAP1. The bound source
revision `09cced01be43cbeaead3267f3a4788d03dd490e1` uses a plain Ethereum REVM lab for body
measurements and real controlled blocks for the transfer fit. Its controlled-block builder
enforces post-Unzen timestamps; its pinned Reth `c096dd3…` already installs and runs per-opcode
Unzen metering. The bound calibration manifest hash matches its retained Git blob.

Therefore the available evidence does **not** support saying the original prices completely
omitted Taiko metering. Dense-workload extrapolation, transfer fitting or program/SDK differences
remain possibilities; this bounded audit does not identify their individual contributions or
recover/replay every original raw input and ELF binding. Independent provenance audit SHA256:
`d7a54cbe5db06f1b3811c69f1751f41bb504cd47cdc0234806f22aa920bb1473`.

## Balanced stack-density observations

A separate24-case catalog (`39b6844a…`) tests PUSH0/POP, DUP depths1/3/8/16 with POP,
and paired SWAP depths1/8/16 at0/4096/262144 bulk pairs. A memory-word counter preserves
17 fixed, distinct full-width seed words at every body boundary, and all final seeds are checked.
The fixed64-site body and input identities remain separate from the preceding36 observations.

The parent has24/24 node deficits and8/24 complete deficits. At262144 pairs, DUP16 has
196,500,938 actual PGU against26,087,760 node and181,029,590 complete PGU; SWAP16 has
261,583,465 actual against98,301,133 node and253,242,964 complete PGU. This strengthens
the shared cheap-operation/composition diagnosis; it does not identify a physical isolated
DUP, SWAP, dispatch or Taiko-meter cost. The independent actual audit hash is
`9d3e95cebcc3755bbcae299a37b8f924ab522f962f6931058d2c0cef22f73383`.

## Empirical normal-dispatch reserve

The development panel now has60 actual inputs and60 same-pattern matched increments.
Freeze immutable parent `bbce9be0…`; for each increment compute the additional normalized
PGU per extra completed user opcode needed to give the increment25% headroom:

```text
required_extra = max(0,
    (5/4 * actual_proposal_delta - parent_node_delta_scaled/scale)
    / extra_completed_user_opcodes)
reserve = ceil_to_multiple_of_8(max(required_extra)) = 64 PGU/event
```

The maximum exact-rational requirement is about58.3843496621 at SWAP16, from4096 to262144.
The60 qualified count deltas exclude Anchor; real candidate replay confirms they equal the
full normal-dispatch count deltas. The selected64 is an **empirical composite conservative
reserve**, rather than an isolated overhead measurement or universal upper bound.
No prospective observation selects it. The original tariff coefficients remain intact.

[Reth candidate `e01c43f`](https://github.com/taikoxyz/alethia-reth/commit/e01c43ff2e79ef75ac9089bb8692093e9c5cbda3)
adds it once for a normally priced dispatched Opcode or ConfirmedSpawn, after route maxima,
so an existing structural floor cannot absorb it. Normal Anchor/child opcodes count too.
Precompile-body events, feature-error pricing and selected-but-undispatched diagnostics do not.
Committed and attempted/discarded prefixes retain their separate accounting.
`normal_dispatch_scaled` is an exposed subterm already inside `operations_scaled`;
it must not be added again by a consumer. The content-addressed candidate is
`c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef`.
The [implementation plan](../superpowers/plans/2026-11-01-normal-dispatch-reserve.md)
records generation, policy and finite validation gates.

## Same-input repricing and prospective validation

All60 original canonical inputs were actually replayed with the candidate quote binary.
The inputs, public outputs, unrelated selectors/prices and raw Unzen results match the parent;
node/full uplift is exactly64 times the independently observed normal count, with existing
scaled arithmetic and boundary rounding. Original actuals are reused without an SP1 rerun.

A further24 untouched validation inputs (`12f81559…`) use fixed128-site bodies and counts
0/8192/196608. Patterns are DUP16/POP, paired SWAP16, full-width XOR, SHL255, SAR255,
CALLVALUE/POP, TIMESTAMP/POP and NUMBER/POP. Their signed shift results, actual header/value,
17 seed words, counts and receipts were independently qualified. Their actuals were opened only
after freezing64. These are new shapes/opcodes/counts, not the fitting cohort or universal coverage.

| Cohort | Actual observations | Parent node deficits | Candidate node deficits | Parent full deficits | Candidate full deficits |
| --- | ---: | ---: | ---: | ---: | ---: |
| Arithmetic |24|24|24|8|0|
| Paired DUP/POP controls |12|12|12|4|0|
| Balanced stack |24|24|24|8|0|
| Prospective validation |24|24|24|10|0|

Candidate full/actual ranges are1.00905..1.17734x on the development60 and
1.00931..1.61555x on the untouched24. All60 development increments and24 validation
increments have at least25% node/full headroom; minima are1.28271x and1.28986x.
Those increment comparisons explain repeatable growth; **they do not erase absolute node
underpricing**. Every N0 control remains in the literal ranking.

Examples of all three quantities under the candidate (each input has one block):

| Case | Node estimate PGU | Complete proposal estimate PGU | Actual proposal SP1 PGU |
| --- | ---: | ---: | ---: |
| ADD full carry,262144 |111,679,768|266,621,598|226,685,516|
| SWAP16,262144 |136,586,253|291,528,084|261,583,465|
| Prospective CALLVALUE,196608 |44,878,559|199,820,390|184,807,541|
| Prospective SWAP16,196608 |100,065,244|255,007,075|232,731,765|

**The node surface remains under actual proposal workload84/84.** Full minus node is about
154.94183M PGU in these controlled single-block inputs, from retained proving-only composition;
that difference is not itself a measurement of actual startup. It has not been allocated to mining.
A roughly21.42x N0 actual/node case remains. Therefore this correction closes the observed
repeatable-operation deficit in the tested increments and the full quotes; it does not close
node admission or justify a universal workload cap. Proposal-aware allocation remains undecided.

The new24 measurements reuse SDK6.3, source-bound ELFb76 and all other frozen artifacts above.
They are execute-only normalized SP1 PGU (`ExecutionReport.gas * 10 // 191`), not generated proofs.
Peak sampled PSS was2,174,916,608bytes under the6GiB guard. Independent cleanup checks found all
96 measurement-owned shared-memory objects absent. The independent actual audit hash is
`eb49f692574803afd5885e799f7237480df9ded864193a533e33f219cb5153a7`;
the original60 real-requote audit is
`198a4cb4a92944d333a9500f8ef7a54b1139a1cff9f8bb106606e903a1334b80`.

## Retained Mainnet sensitivity

The same50 retained Mainnet proposals /9,600 blocks were natively repriced using the matched
`legacy-unzen` execution profile. Summing proposal `node.block_local_known_pgu` values
(one ceiling after each proposal's scaled block sum) gives84,759,814,532 parent and
92,349,395,460 candidate PGU, a rise of8.95422%. The literal sum of individually rounded
block values (`sum_of_block_known_pgu`) is84,759,816,672 and92,349,397,600 respectively;
both conventions are retained. Full sum rises4.99632%,
with maximum proposal full uplift7.31455%. Workload-weighted full/retained-actual changes
2.17064x to2.27909x; complete deficits remain0/50. Against those retained actuals, literal node deficits change34/50 to21/50. The largest
remaining actual/node ratio is proposal45209:1,069,360,738 /891,258,762, about1.19983x,
with a178,101,976-PGU shortfall; the node sum covers the population only in aggregate, not
proposal by proposal. These ratios retain the historical-program caveat below.
Other prices, inputs, outputs and Unzen scores match. The initial `evm-gas-only` diagnostic is retained separately, rather than mixed
into this matched-profile comparison.

The retained historical actuals still bind SDK6.1 / ELF416f5a47; no historical SP1 rerun occurred.
These numbers measure price sensitivity and a cross-program workload diagnostic, **not** strict
current-ELFb76 accuracy certification or node closure. The roughly2x historical conclusion is
not overturned, although the measured5% additional overestimate remains an explicit tradeoff.
This cohort's inputs are separate from the synthetic84-case fixture.

## Current boundary and next research

The reproducible generators, qualification checks and usage remain in Raiko; immutable inputs,
raw observations and source identities belong in Atlas. No production limit, activation mode,
header difficulty, global discount or future fork policy is changed. All three source PRs remain
pending/do-not-merge; no guest artifact is rebuilt or relabeled as current.

Next, use equivalent-input controls to test growth with block count and composition/failure
boundaries. Distinguish the genuine proposal-once remainder from any remaining per-block or
per-operation growth before considering proposal-aware allocation. In particular, keep literal
actual/node ratios and both deficit flags even when the full estimate covers the input.

## Verification and limits

Independent qualification checked every actual ordered operand, modulo2^256 result, target/counter
count, full opcode map, receipt, code preparation and exact gas decomposition. Four native negative
controls were rejected without published inputs; a commutative operand swap was detected by the
independent oracle even though its receipt and opcode counts could pass.

The independent actual audit rehashed the frozen producer sources, canonical retained inputs,
public outputs and raw reports; recomputed `ExecutionReport.gas * 10 // 191`; checked all three
quantities against the scorecard; and verified48 retained attempts (24 screen+24 actual). It also
verified all96 owned shared-memory objects absent in the host namespace after execution. Peak
sampled PSS was1,704,820,736bytes, below the6GiB guard. Its report SHA256 is
`9bf388edd49dd17008916b040f76279f7d2ec76a2cefc8356d3ad146f084a0ec`.

This cohort does not establish a global maximum, a per-block actual workload, production capacity,
or closure of the node's proposal-once observability gap. The candidate and finite sensitivity/validation results above do not establish those stronger claims.
