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
throughput measurement. No guest, dependency pin, price, discount or admission limit changed.

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

The next finite experiment separates PUSH0/POP, DUP depths1/3/8/16 and paired SWAP depths1/8/16.
Its counter stays in memory, keeping the actual 17-word stack unchanged at every body boundary.
New source/inputs and actual measurements remain separate from these frozen 36 observations.

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
or closure of the node's proposal-once observability gap. No pricing correction has been chosen,
and historical sensitivity has not been rerun for a nonexistent candidate tariff.
