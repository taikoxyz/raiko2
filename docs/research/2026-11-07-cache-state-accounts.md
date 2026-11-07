# Cross-transaction account-cache concentration

The current model covers both measured inputs in this finite panel. At the same
21,637,563 charged block gas, concentrating the addresses raises complete SP1
workload from302,539,274 to946,126,714PGU. The successor's literal node estimate
is3,790,845,145PGU, leaving2,844,718,431PGU of headroom. No tariff changes follow.
This is a selected-distribution experiment, not a global block-workload bound.

## Question and fixed workload

The backing `State.cache.accounts` survives transactions inside one Raiko block;
the current Raiko call site creates a fresh State for each block. Journal
finalization takes the separate per-TX Account map. The lifetime of a generic
reusable executor API can be wider, so this is not an automatic cache-reset
contract for every caller.

One block contains512 users calling512 predeployed215-byte runtimes. Each user
reads eight globally disjoint, initially absent BALANCE targets and checks that
every result is zero. There are4096 targets, empty calldata/access lists, no
memory expansion, no storage and no refunds. Each user consumes41,934gas within
its50,000 envelope; the Anchor consumes167,355gas. All513 transactions fit the
unchanged31M header, with21,637,563 charged gas below the30M severity screen.
Predeployment is outside this block's measured lifecycle.

The neutral target vector is SHA256 of
`b'cache-state-absent-control\x00' + index.to_bytes(4, 'big')`, truncated to20bytes,
for indices1..4096. There is no filtering or collision retry. After independently
reading this reference's own CacheAccount specialization, exactly one successor
uses the existing Address20 helper with reference G=`0xef287aafd5c455bb` and a
six-byte common prefix. It changes the target/code immediates only. No third
input, size sweep, seed search or repeated measurement was performed.

## Complete measurements and literal estimates

| Input | Raw SP1 gas | Normalized actual PGU | Literal node PGU | Full proposal PGU | Actual minus node PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| Neutral reference |5,778,500,151|302,539,274|3,780,228,313|3,957,654,344|-3,477,689,039|
| Sole successor |18,071,020,245|946,126,714|3,790,845,145|3,968,283,464|-2,844,718,431|

Normalization is integer `raw_gas * 10 // 191` under the retained SDK6.3 report
contract. Each denominator is the literal sum of integer block-node estimates;
the one-block node value is not a completed proposal quote. No startup cost is
subtracted. Raw Unzen score169,222,586 is identical between these inputs and is
kept separate from PGU.

The whole-input actual increase643,587,440PGU is larger than the node increase
10,616,832PGU. This does **not** establish literal block underpricing: cold
BALANCE's existing Maximum route alone contributes approximately3.594BPGU,
and all node components participate in the comparison. The new Account reserve
must not be credited with covering the entire actual increment by itself. The
paired difference is not an isolated CacheAccount cost coefficient.

Node values differ only in Account work units41,660→124,604 at128PGU/unit.
All other node numerical components are equal. The full-proposal increase has
an additional12,288PGU from its actual MPT selectors. The constructed neutral
control's approximately12.5x node overquote and successor's approximately4.0x
remain visible; neither input is a historical-transaction usability sample.
Normal historical pricing is unchanged because no model change was made.

## Canonical input and diagnostic qualification

Independent checks recover every signature, verify user nonces0..511 and all
512recipient runtimes, authenticate4096absences, rebuild full516→517account
worlds and transaction/state roots, and check receipts, routes, gas, Kona payload
and256ancestor-header encodings/hashes. All512 user unsigned/signed transaction
bytes, the Anchor's signed bytes and the Kona payload are identical between
inputs. Changed code lives in authenticated prestate, outside signed user
payloads; code/witness/root/header/canonical input changes are independently
recomputed. The differing public output is expected.

| Identity | Reference | Successor |
| --- | --- | --- |
| Canonical input SHA256 |`781a6145d058c7abfdce3bfd68f39e913e7af0cd8ced418dfb922232fbd6dbdf`|`05401bda6c6846ff8df12401dd566bbde197f82bfafea5c9fc50c096b4eacd39`|
| Raw SP1 report SHA256 |`2237baaf43102d4bd0829fd058ef43375d4b2227b173901730b2eacee7d29180`|`557d8fa49cd1ef38991e7ddf48ac4b68f55fb534e8ee683603429f777b02fe79`|

Both canonical inputs are687,003bytes. Actual artifact identities remain:
ELF `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`,
SDK6.3 observer `dfaee7ac41099df02e8a66d9b15b6c4b53d539d1a7e051a67b7180c64d6a0992`,
quote `ef8f15a9049b0258cb1c141dab367c63ecc314c1c7ff6582393d36bcb8a86e56`,
model `af30c61656fd456ab290a837b44c181e4ff54e575a949d3579f07df20659a949`.
The constructor is the retained f261-dirty build; quote is the reviewed
24791-dirty/Reth03a8-dirty build, source-equivalent to later commits. Later clean
refs do not relabel these binaries. Production SDK/program parity is unverified;
SP1 here is execute-only estimation, not a generated proof or GPU timing.

The exact ELF has a distinct160-byte CacheAccount entry path. Reference local
qualification closes20 calls, including eight target calls, with own
S=`0xd7b3131e90f4abdd` and G above; its eight observed hashes differ. The
successor's capped local readback has the same own G, with eight target hashes
all equal to G. Independent raw-register reconstruction verifies caller, key,
map, stack frames, seed, hash and return association. Computing4096 conditional
catalog hashes is not observing4096placements. TX identity, later-map identity,
final population and whole comparison count remain UNKNOWN.

A private postreader initially used the entry-call return address for both seed
and post-helper header. Actual ELF inspection established different live return
addresses; a separate corrected epoch preserves all other strict joins and
exclusions. Tests derive both addresses from ELF machine words, and an independent
raw reconstruction matches the corrected output. Original reader/raw/zero-row
output remain retained. No measurement or observation plan was rerun to repair
this analysis-only error.

The successor completes1,046,360,642canonical instructions while optional
observations stop at400M. Complete actual PGU is valid independently of this
incomplete diagnostic prefix. Each input has exactly one constructor, one quote
and one SP1 execution, with6GiB/600s actual caps. Peak process-tree PSS is2.08GB
and4.55GB, with21.02s and41.01s wall time. Original-host receipt-specific producer
and all eight owned SHM objects are absent after completion; no cleanup signals
or manual unlink were needed. No guest, ELF, sampling SDK or observer rebuild
was performed.

## Reusable interface and remaining scope

The [benchmark guide](../../bin/zkgas/bench/README.md#cross-transaction-account-cache-requests)
documents the committed `cache_state_accounts.py` producer, standard
ProgramRequest/catalog outputs and explicit raw-address API. Existing `workload.py`
performs screen/retained-input measurement; no additional product runner is
introduced. Pure decoder details remain diagnostic artifacts. Canonical inputs
and original reports are retained for a separate data-only Atlas fixture.

Access-list-prewarmed DB loads are unmeasured here; this cold-only result must
not be extended to that composition. The separate
[warm-first panel](2026-11-07-cache-state-warm-accounts.md) measures a high-priority
literal-node and full-proposal deficit under the same model.

Neither input has a positive literal-node or full-proposal deficit, so neither
meets the >2BPGU/<=30Mgas high-priority screen. Other block-long containers,
external validation/fee work and broader workload composition remain distinct
follow-ups. This finite panel does not close them or establish a global30M bound.
