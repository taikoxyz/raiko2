# Block-local storage transitions: measured cumulative underpricing

The retained `02223227` model misses a high-priority storage-transition workload:
**3,550,141,992 actual SP1 PGU against 998,599,458 literal node PGU**, at
23,541,753 total charged EVM gas. Full proposal estimation is also low. The
deficits exceed the experiment's >2B PGU / <=30M charged-gas priority screen.
This is an observed input, not an extrapolation to a full block.

The previous [single-block pair](2026-11-07-transition-storage-updates.md)
remains covered, with its intended concentration unrealized. This separate
two-input panel uses the permanent
[genuine prefix helper](../../bin/zkgas/bench/README.md#genuine-empty-user-prefix-blocks).
No third input, seed/count sweep or same-input measurement retry follows.

A later [read-only storage panel](2026-11-07-prefixed-storage-read-cache.md)
finds a separate high-priority deficit in the `3e335de2` correction below.
This document retains the earlier model's formula and validation results;
those results do not certify the later read-through query scope.

## Genuine two-block composition

`counts_by_block=[0,64]` creates a real Anchor-only block, then an Anchor and
64 ordinary CALL users. Each user updates its next 64 distinct, initially nonzero
body slots from 1 to 2 and advances the same owner's phase slot. The recipient
starts with 4,097 slots; phase starts at 1 and ends at 4,097. Setup is a prestate
fixture condition, outside this proposal. The user kernel has no calldata,
access list, authorization, CREATE, refund or restore-to-original path.

The initialized body key is `k(i)=(i<<48)|(L1<<64)|(L3<<192)`, for i=1..4096,
with L2=0. Reference fixes L1=L3=0. Before construction, the protocol permits
one successor selecting L3 from the earliest independently qualified reference
body-key observation. It selects `0xe57d314eeb7f77cf`; L1 remains zero.

The actual charged block gas is `[167355,23374398]`, summing to 23,541,753.
The two Anchors use **167,355 and 107,134** gas respectively; the first receipt
is not copied to the second block. Each user consumes 363,551 gas with a 364,551
transaction envelope and 308-byte runtime. All 66 signed transactions and full
Kona data sources are equal across the pair, verified afresh. Runtime masks,
concrete keys, authenticated storage/world roots and witnesses differ.

## Complete workload versus both estimates

| Input | Actual SP1 PGU | Literal node PGU | Full proposal PGU | Actual minus node | Actual minus full |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reference, L3=0 | 363,477,553 | 998,599,458 | 1,302,047,321 | -635,121,905 | -938,569,768 |
| Sole reference-G successor | 3,550,141,992 | 998,599,458 | 1,301,217,353 | +2,551,542,534 | +2,248,924,639 |

Actual is `floor(raw_SP1_execution_gas * 10 / 191)`: raw values
6,942,421,276 and 67,807,712,059. Node is the literal sum of individually rounded
block-known PGUs, `[7320715,991278743]`. Full proposal is computed separately.
All prefix/Anchor/startup costs remain included. Raw Unzen difficulty is not PGU.
The successor actual/node ratio is 3.55512; actual/full is 2.72832.

Full quote decreases by 829,968 PGU, entirely in second-block prove-MPT pricing.
Proposal-once objects are actually equal. The approximately 3.187B actual
difference between these opened controls is not an isolated per-map coefficient.

## Local observations and source interpretation

Independent raw-register reconstruction closes 63 local chains, including
62 distinct body keys, for each input. The final capped parent interval is
excluded even when its return is present. Actual ABI/chronology, fixed instruction
distances, map/key/result pointers, raw-limb order and full-hash arithmetic are
checked independently of the campaign reader.

Both inputs' observed own shared G is `0xe57d314eeb7f77cf`. The reference's
62 observed body full hashes are distinct and nonzero; the successor's 62
observed body full hashes are **all zero**. For this ELF's fixed32 specialization,
`H=F(F(L0 xor S,L1 xor G) xor L2,L3 xor G)` and
`F(x,y)=low64(x*y) xor high64(x*y)`. L3=the same call's G makes the final
multiplication zero, and actual register values confirm that local result.

This establishes sampled local concentration. It does not establish all 4,096
placements, whole-map comparison count, final population or transaction/owner
identity from a pointer. Equal observed G in this pair is not a universal seed
stability guarantee. The complete canonical instruction counts are 316,321,942
and 4,333,066,772. The successor's optional observation stops at 400M instructions;
that capped observation does not truncate its separately complete canonical PGU.

Source inspection of the frozen workload and ordinary proposal paths establishes
a fresh EVM State/CacheState/TransitionState per block. Semantic storage values
carry through authenticated roots/witnesses; these physical maps do not carry
between block executors. Within a block, CacheAccount storage and TransitionAccount
storage accumulate across user transactions. The existing `storage_journal`
population is transaction-local, so its reserve misses that cumulative dimension.

## Selected correction and retained-input validation

The source/design review supports a separate block-local `storage_transition`
reserve, with inherited tariffs/discount/defaults unchanged. For actual state
owner o and finalized transaction t, J is the physical journal key set and X its
`is_changed()` subset relative to that transaction's original values. Keep each
prefix's per-owner union K of all J, including earlier read-only keys. Merge the
whole current receipt before computing:

```
N = |K[o]|; M = largest raw-key low32 group
C = ceil((N + 7*M) / 8)
U += C * 2 * |X[o,t]|
additional PGU = 384 * U
```

The existing 384 PGU/unit tariff is reused. Two changed-slot passes correspond
to the forward CacheAccount and TransitionAccount update paths. Read/noop X=0
can legitimately grow N with zero new work. Cross-transaction restoration to
the block-original value still counts a changed transaction slot and its entry/
removal work. Committed and attempted populations are independent; block reset,
Disabled behavior, sticky incompleteness and proposal-once zero remain required.

For the target owner alone, `N=M=1+64*t`, `|X|=65` for t=1..64. The policy
predicts U=17,313,920 and +6,648,545,280 PGU. This gives successor node at least
7,647,144,738 and full at least 7,949,762,633 before other owner contributions.
These are **target-only arithmetic projections**, not fresh whole-input quotes.
The reference receives the same conservative target term; its artificial
overquote remains part of the evidence.

The independent source and behavior gates pass for candidate model
`3e335de20582fca48f84b3df1b2ecd8d8d7281de02442ab35a134e4635431d73`.
Fresh retained-input quotes give:

| Input | Original actual SP1 PGU | New literal node PGU | New full proposal PGU | Actual minus new node |
| --- | ---: | ---: | ---: | ---: |
| Genuine-prefix reference |363,477,553|7,647,153,954|7,950,601,817|-7,283,676,401|
| Sole reference-G successor |3,550,141,992|7,647,153,954|7,949,771,849|-4,097,011,962|

The observed total new work is17,313,944units:16 in block0 and17,313,928 in
block1. That is the target owner's17,313,920 plus24 other-owner units, adding
exactly6,648,554,496PGU to both node and full with zero proposal-once contribution.
The successor is covered at2.15404x node/actual and2.23928x full/actual. The
artificial reference's21.03886x node overquote remains visible; it is not normal
precision evidence. The original single-block pair is also requoted and covered,
at7,642,552,232 node PGU each, about20.2x actual.

All82 retained inputs/9634 blocks pass exact non-new-component/selector/public/
Unzen equality and new384PGU-unit arithmetic, including integer node sums and
independent full ceilings. All29 numeric actuals are covered by full estimates;
25 are covered by literal node estimates. Four smaller node deficits remain,
maximum153,346,391PGU, below the >2B priority screen. Three controlled rows stay
explicitly unmeasured. Original actuals are reused, without SP1 resampling.

The50-proposal/9600-block historical sensitivity cohort gives:

| Metric |0222 total PGU |3e335de2 total PGU | Added PGU | Increase |
| --- | ---: | ---: | ---: | ---: |
| Literal integer-block node sum |92,857,351,648|92,887,659,232|30,307,584|0.032639%|
| Full proposal sum |160,000,862,669|160,031,170,253|30,307,584|0.018942%|

All50 proposals/9600blocks have positive additions. Maximum proposal-node uplift
is0.066003%, maximum block-node uplift0.091275%; median/p95 block additions are
3,072PGU and the maximum33,792PGU. This does not materially change the retained
historical quote scale. Older SDK6.1/416f historical data is price-sensitivity
evidence, not b76 accuracy certification or an admission-limit guarantee.

The model/collector has280 EVM and62 Python tests, complete independent source
review and nine independent behavior tests. The host bridge has37 trace and140
default-CLI tests plus five strict production clippy checks. The unchanged genuine
192-block fixture has a separately verified1,622-unit/622,848PGU new component,
with original identity and inherited goldens retained. All82 numerical results
are independently recomputed; no whole-workspace/guest/SDK verification is claimed.

The forward reserve is empirical, not a universal PGU/map bound. End-of-block
bundle construction, read-only cache work, other layouts and custom prefilled/
backend/pre-system paths remain explicit follow-ups. `HashMap::clone` does not
rehash; no such cost is invented to justify the reserve.
The next finite candidate is many distinct reads through block-long PlainStorage
with few final changed slots. Source analysis identifies the missing physical
scope but has not measured a new deficit or selected another tariff. Dispersed
low32/full-hash-collision short comparisons remain a separate follow-up.

## Retained identities and limits

| Artifact | Reference SHA256 | Successor SHA256 |
| --- | --- | --- |
| Canonical input | `b101c24b1b8daf3a0354942edebfd9cee185281a2c0dd5dd45737ba83557364b` | `9a0ab0d7f405adc23f4a4e569ad44b65b4e96183c498c3e9fbe855beea8dd596` |
| Raw SP1 report | `fee8808e3a3dc00749ca049190df0413312805b9b41bbb2e8949691268fd6fc7` | `362a5e19ea671f0c93398963f66e04d10eb33ef1168967c49123102de6ead6da` |

Each input has one construction, quote and SP1 execution. Peak process-tree PSS
is about 2.31GB/4.56GB and wall time 20.86s/149.89s, below the existing
6GiB/600s caps. Independent original-host checks find both producer PIDs and
all eight receipt-owned SHM objects absent, without signals or unlinks.

Native executable SHA `1368d40d` retains its actual `b3f96424-dirty` source
identity; quote `9ba9b8f5` retains `dbe3563-dirty / Reth c164851-dirty`, with
current-source equivalence checked separately. Later clean commits do not
relabel either build. The retained baseline model is
`022232270da81c96dd9d7a12899d52ac011f46144d7f90d778a585aaeca5b7bd`.
The new host quote is9,826,640bytes/SHA
`c9ba73103308e208f223349b690f5a1e45a90a89b573ea357f906e228e2bf53a`,
actually built from Raiko`08fbc3eb-dirty`/Reth`182d5e57-dirty` candidate sources,
not relabeled to a later clean pin. The fixed192-block count assertion is a
test-only addition with byte-identical production source. The source/ABI/field
and numerical gates are separate from the old native/SP1 program identities.
The frozen ELF is `b76baf2e`, observer `dfaee7ac`, SDK6.3. No guest/SDK/ELF/
observer rebuild, current-production parity, proof, GPU timing, untouched holdout
or global workload bound is claimed. The closed single-block Atlas fixture is
unchanged. This pair is retained in a separate
[immutable data revision](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-prefixed-storage-transitions-v1),
metadata commit `d9ef9d91027e4424328c7758209e40b1e228161d`:
63 regular data members, 2,315,521 compressed bytes, SHA256
`9fec0075be08d2dbc0ce7d0bb35b6db9a9d43732ef7435dc60692e7bb3735530`.
Original bytes and documented deletion projections are separately identified;
no executable tooling or private source maps are bundled. Independent data/source
and local README/CI checks passed, followed by actual remote-download README and
unchanged CI-body checks. The exact-head dedicated fixture job also passed;
packaging neither remeasured workload nor supplies a pricing correction.
