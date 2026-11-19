# Access-list construction pricing

A single constructed block using 14,782,761 EVM gas measures 5,773,513,175 SP1
PGU. Model923 quotes only 142,698,613 literal node PGU and 297,640,444 complete
proposal PGU. The respective deficits are 5,630,814,562 and 5,475,872,731 PGU,
crossing the research worklist's >2B-PGU deficit within a <=30M-gas block rule.

The successor model389 adds an independently reported access-list construction
component. Replaying the same input for its new quote gives 11,465,844,853 node
and 11,620,786,684 complete-proposal PGU, approximately 1.986x and 2.013x actual.
The original SP1 execution is reused. This result closes the demonstrated large
deficit; it is not a bound on arbitrary block composition or all hash tables.

## Mechanism and observation

Before opcode execution, non-legacy transactions construct per-owner warm
`HashSet<U256>` collections from their declared access lists. Logical resource
uniqueness and signed input bytes already have linear prices. The persistent and
transient journal reserves do not account for this separate collection's
construction. The constructed user program is only two `PUSH1` instructions and
`RETURN`, with empty calldata and no persistent storage reads or writes.

The retained generic-hash catalog uses a selected shared word G and distinct
keys `G | (G << 64) | (i << 224)`. Under its own matching guest shared word, the
source-derived hash cancels the per-map parameters. Keys share the first 28 bytes
compared in the guest's little-endian U256 representation. This is separate from
the journal's fixed32 hash and from MPT key hashing.

G is global within an initialized guest execution, while each map has its own
state. Input decoding and allocation history can change G across inputs. Earlier
G values are hypotheses for a new input. Valid executions that do not realize the
selected distribution remain controls; neither equal key count nor equal signed
transaction length certifies seed stability. Each measured input has its own
bounded observation qualification.

For the large realized input, 31 linked calls confirm its owner, key, frame,
shared word, zero hash body and expected final hash. The 400M-instruction observer
prefix stops during construction: 1,693 insert hits, 32 retained samples, 31
qualified linked calls, and 1,692 observed commits. Its 1,431,205 equal-candidate
counter is a global prefix counter, not the complete comparison count. The
canonical execution and PGU report are complete. The computed 7,680-key catalog
is not 7,680 dynamically observed placements.

## Block-observable supplement

For each transaction and each declared owner, count raw occurrences in bins of
the storage key's **numeric low32**, equivalent to the last four bytes of a
big-endian B256. With raw bin sizes n:

```text
construction_pairs = sum over transactions, owners and bins of n * (n - 1) / 2
extra normalized PGU = 384 * construction_pairs
node increment = 384 * committed construction_pairs
prove block increment = 384 * attempted construction_pairs
proposal-once increment = 0
```

Repeated owner items share bins within one transaction; the next transaction
starts fresh. Duplicate raw keys deliberately increase the reserve. A scratch
ordered map computes the receipt and is dropped before EVM execution. This does
not add another rolling journal or change logical resource deduplication.

The receipt is separate from `access_list_items` and physical journal work.
Node reports use committed scopes; prove reports include attempted scopes.
Missing nonempty capture, repeated or mismatched receipts and arithmetic/domain
failures produce diagnostics. Off mode skips the scratch work. Proposal-once
pairs and contribution are zero. Both reports expose
`access_list_construction_pairs` and `access_list_construction_scaled`; the latter
is added once outside operations. The selector has an independent 10^12 domain.
The serialized scaled contribution multiplies normalized PGU by 10^18. The pair
selector uses signed access-list metadata and does not need a proposal witness or
the guest's future shared word.

At K=7,680 the raw pairs are 29,487,360, adding 11,323,146,240 PGU. This is an
empirical conservative supplement informed by the retained measurements and the
existing 384-PGU comparison reference. It is not a universal hash-table or PGU
upper bound. All inherited numeric prices, route records and discount treatment
are unchanged; the deterministic generator authenticates immutable parent923.

## Retained finite panel

Eight distinct canonical inputs have eight original and eight successor quotes.
An additional old-native empty bridge makes nine constructor executions, not
nine inputs. Five actual executions completed once; three remain unmeasured.
All rows use `evm-gas-only`, so legacy Unzen caps do not screen the experiment.

| Case | Block EVM gas | Actual PGU | Old node PGU | New node PGU | New proposal PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| Empty | 188,361 | unmeasured | 7,513,717 | 7,513,717 | 162,455,548 |
| Micro, four raw keys | 198,361 | unmeasured | 7,590,069 | 7,590,453 | 162,532,284 |
| Reference64, unrealized | 312,361 | 161,458,895 | 8,655,989 | 9,430,133 | 164,371,964 |
| Dispersed64 | 312,361 | unmeasured | 8,655,989 | 8,655,989 | 163,597,820 |
| Reference1024, unrealized | 2,136,361 | 167,736,345 | 25,551,989 | 226,681,973 | 381,623,804 |
| Aligned1024, realized | 2,136,361 | 266,428,229 | 25,551,989 | 226,681,973 | 381,623,804 |
| Reference7680, unrealized | 14,782,761 | 210,667,979 | 142,698,613 | 11,465,844,853 | 11,620,786,684 |
| Aligned7680, realized | 14,782,761 | 5,773,513,175 | 142,698,613 | 11,465,844,853 | 11,620,786,684 |

The two remaining measured node deficits are 152,028,762 and 39,746,256 PGU;
startup is not subtracted from either denominator. All five successor proposal
quotes cover actual. The unrealized large control has approximately 54.43x node
overquote and is retained explicitly. This is a conservative false positive on
a constructed concentrated layout, not evidence of ordinary transaction usability.
The measured panel has no remaining >2B/<=30M deficit; arbitrary packing remains
a separate question.

The retained 2026-10-27 through 2026-10-29 historical panel was also re-quoted:
50 proposals and 9,600 distinct blocks. All new pair components are zero and
every other nested quote object and public output matches its predecessor.
Literal summed-block node PGU remains 92,561,066,464; proposal PGU remains
159,704,576,845. Whole-sum ceilings and sums of per-block ceilings are checked
separately. This establishes pricing sensitivity for that panel. Its older
416f/SP1v6.1 observations do not certify current b76 precision. Raw Unzen scores
are a separate unit and are not compared directly with PGU.

## Tools, identities and verification

The [immutable Atlas fixture](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-access-list-construction-v1)
retains the eight inputs, original measurements and both quote snapshots. Its
published asset passed a fresh remote download and compressed/decoded checksum
verification; the existing fixture CI assertions and literal README workflow also
passed against the uploaded asset. This is fixture publication, not deployment.

The [raw-key producer and generic hash helper](../../bin/zkgas/bench/README.md#explicit-raw-access-list-storage-keys)
retain explicit key order and duplicate occurrences. The existing declarative
`workload.py` harness now binds and retains an optional
[observation plan](../../bin/zkgas/bench/README.md#frozen-observation-plans-in-the-workload-harness).
Canonical PGU validation remains independent of optional trace qualification.
No family-specific launch controller is required by that interface.

Actuals use ELF `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`
and frozen SDK6.3.0, with exact normalization
`floor(ExecutionReport.gas * 10 / 191)`. The largest raw gas is 110,274,101,646.
All 20 owned shared-memory objects from the five runs are absent on the original
host. Peak PSS is 4,550,495,232 bytes; maximum guard runtime is 162.045 seconds,
within 6GiB/600s. Diagnostic replay time is not GPU proving time.

The new quote executable SHA256 is
`5ca063730658e15e83cb2f95e275f05eec98a86097dd9be8409638c7472c76fd`.
Its actual build retains Raiko `3e6e0c59`/dirty=true and Reth `cac269da`/dirty=true.
Reviewed source-equivalent commits are Reth `75d7f858` and Raiko `5f3795c7`;
they do not relabel that earlier binary as a clean build. Full model identity is
`389ca65f21fb17b04121e1b0915662f85e4b617b4c12b3b2c949faf591eb5493`;
its JSON file SHA256 is
`2066bb1830322150a26b5f89ae8338bcce75f762eb99da1c9ac642bfa63ec260`.

Independent source and behavior checks passed, including a 241-test EVM overlay
(235 implementation tests plus six independent cases), 46 generator tests, 31 trace and 116
CLI tests, 53 workload-harness tests, and independent checks of all 58 new raw
quotes. Fresh host outputs preserve the old binaries and cached dependencies.
Workspace formatting and locked offline no-deps metadata pass. Full workspace
Clippy is blocked by the missing offline Git checkout for the new pin; it is not
claimed passed. No guest/SDK rebuild, new ELF or additional SP1 sampling was
needed for repricing. The branches remain experimental Draft/do-not-merge.

Short-prefix distribution, control/probe/rehash costs, multiple large access-list
transactions and other containers remain follow-ups. Neither this supplement nor
the historical zero delta certifies a global block workload bound. See the
[evaluation worklist standard](../zkgas-evaluation-standard.md#gas-bounded-priority-and-pricing-complexity).
