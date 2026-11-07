# Warm-first account-cache composition

Model `af30` underprices the sole reference-G successor in this finite panel. One
valid block consumes 29,669,999 EVM gas and measures 5,027,210,167 complete SP1
PGU, against a literal node estimate of 1,354,594,058. The positive difference
3,672,616,109 exceeds the agreed >2B PGU / <=30M gas priority screen. The full
proposal estimate also underprices it. The measurements below preserve that
original model. The separately validated `02223227` supplement at the end
covers this demonstrated deficit.

## Why warm-first is a separate composition

An access list marks an account warm for EVM gas accounting. It does not by
itself populate the backing `State.cache.accounts`. A first BALANCE can therefore
cost 100 EVM gas while still loading the account into the block's cache. That
cache survives transaction boundaries in the selected block executor; the
separate Account journal is finalized per transaction. The previous
[cold-account panel](2026-11-07-cache-state-accounts.md) cannot certify this
composition using the larger cold-opcode reserve.

Each of 86 users calls its own predeployed 3,335-byte guarded runtime, reading
128 globally disjoint, initially absent BALANCE targets. Its signed access list
contains precisely those 128 addresses, each with an empty storage-key list.
Calldata and value are zero. User kernels perform no storage operations and
have no refunds, CREATE or authorizations. All 11,008 guarded reads return zero
successfully.

Per user, gas is 328,200 intrinsic + 12,800 warm account access + 2,054 ordinary
opcode gas = 343,054, inside a 400,000 envelope. Adding the 167,355-gas Anchor
gives 29,669,999 charged block gas. All 87 transactions satisfy admission under
the unchanged 31M header. Predeployment is outside this measured block.

The reference uses the fixed public structural parameter G=0, with address
bytes `A5 || zero[5] || index_LE16 || G_LE64 || 1_LE32`, indices 1..11,008.
It is neither randomized nor certified dispersed. After independent reference
readback, the sole successor substitutes the qualified Anchor-local CacheAccount
G=`0xdcbc0be5baaa417f`. Shape, envelopes and every other request field remain
fixed. There is no third input, seed search, count sweep or measurement retry.

## Complete actuals and literal prices

| Input | Raw SP1 gas | Actual PGU | Literal node PGU | Full proposal PGU | Actual minus node PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| Fixed-G0 reference | 6,467,863,636 | 338,631,603 | 1,354,594,058 | 1,518,988,233 | -1,015,962,455 |
| Sole successor | 96,019,714,195 | 5,027,210,167 | 1,354,594,058 | 1,519,000,777 | +3,672,616,109 |

Actuals use integer `raw_gas * 10 // 191` from the retained SDK6.3 report
contract. The denominator is the literal sum of integer block-node estimates.
No startup cost is subtracted and raw Unzen scores are not treated as PGU.
The successor's full-proposal deficit is 3,508,209,390 PGU, so completing the
proposal quote does not remove this gap. This is measured at the actual gas
budget, without extrapolating a smaller input to a full block.

All node pricing fields are equal across the pair. Existing Account work units
are 4,293,566, raw AL items 11,008 and raw address pairs 699,008. Their reserves,
opcode/resource/payload/fixed costs and every other existing term participate
in the literal comparison. The full-proposal increase is 12,544 PGU from MPT
selectors; its proposal-once object is identical. The whole actual difference
is not an isolated cache-comparison coefficient: changed addresses also affect
access-list, journal and other account paths.

## Canonical data and bounded diagnostic evidence

Independent checks recover all 87 signatures and nonces, validate each ordered
access list against the actual guard code, authenticate all 11,008 absences,
rebuild the complete 90-to-91 account worlds and storage/state/transaction
roots, and check receipts, gas, fees, code, minimal witness and Kona payload.
There are 122 selected witness nodes and 88 unique code bodies. The compressed
one-blob payloads are 29,935 and 29,843 bytes, below the unchanged 126,972-byte
bound. All 86 users' unsigned and signed bytes change; the Anchor's signed
bytes happen to remain equal. Both inputs have a 263,024-byte canonical signed
transaction selector and a 263,121-byte decoded user-RLP payload. Those are
different metrics; equal lengths do not mean equal contents.

| Identity | Reference | Successor |
| --- | --- | --- |
| Canonical input SHA256 | `b7636424e5d3b91dcde7b80e9575a2c4cddb9571736b55434eb186b508fe2cff` | `bf74ff68488f18f1efef93153ebdf53cb7c712e8b029da2fe438185eeceb03ba` |
| Raw SP1 report SHA256 | `579df9187b1be356bebc7e5d87c2ad7b332fa7f6d469e47cd4fffc7d48d85118` | `b8a291ecb206f7a1cf7fca18e6267a6967733253da31d4945211edd159ca7307` |

Both canonical inputs are 1,056,123 bytes. Actual artifacts remain ELF
`b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`,
SDK6.3 observer `dfaee7ac41099df02e8a66d9b15b6c4b53d539d1a7e051a67b7180c64d6a0992`,
quote `ef8f15a9049b0258cb1c141dab367c63ecc314c1c7ff6582393d36bcb8a86e56`
and model `af30c61656fd456ab290a837b44c181e4ff54e575a949d3579f07df20659a949`.
The per-TX AL constructor `9b9e605ace5098ccc5e7dd0ebda6867ce34c246683caf78309f711309a39978c`
was built from the reviewed 350f3dd5 dirty snapshot with retained lower libraries;
quote retains its actual 24791-dirty/Reth03a8-dirty identity. Default-feature
source/ABI equivalence was separately checked. Later clean commits do not
relabel either executable. These are execute-only estimates, not generated
proofs, GPU timings or production SDK/program parity certification.

Independent raw reconstruction closes 11 CacheAccount calls in each input,
including its authenticated Anchor address. Both local Anchor chains have
own S=`0xd7b3131e90f4abdd`, own G above and full H=`0xefed74a3d6c10c5e`.
The returned State pointer plus the actual ELF field offset identifies the
observed local cache map. The successor's observed local G matches the selected
reference G; this is not evidence borrowed from another input.

Zero target calls have complete hash chains: the shared helper's first-64
capture window ends before the first target. Four foreign-parent, 48 missing
shared-word and one capped-tail intervals are excluded in each readback.
All-target dynamic hashes/placements, transaction identity, final population,
second-map identity and whole pressure remain UNKNOWN. No extra observation,
weakened join or new input repairs those limits. The complete workload deficit
stands independently of this incomplete mechanism diagnostic.

The reference completes 292,477,011 canonical instructions; the successor
completes 6,056,329,637. Optional observation stops at 400M for the successor,
while canonical execution finishes normally. Each input has exactly one native,
one quote and one SP1 execution. Peak process-tree PSS is 2,196,784,128 and
4,555,841,536 bytes, with 19.40 and 138.70 seconds wall time, below 6GiB/600s
limits. Independent original-host readbacks confirm both producer PIDs and all
eight owned SHM objects absent; no manual signals or unlink were needed. No
guest, ELF, SDK or observer was rebuilt for these measurements.

## Reusable tools and next step

The [per-TX AL guide](../../bin/zkgas/bench/README.md#per-transaction-access-lists-and-warm-first-cachestate-cases)
documents `program.transactions.access_lists` and the public
`cache_state_warm_accounts.py` producer. They use the standard ProgramRequest,
catalog, screen and retained-input measurement interfaces. Existing no-AL and
shared-AL output bytes remain compatible; no new guest-input schema or runner
is introduced. Canonical inputs and original reports are retained for a
separate data-only Atlas fixture.

The original finding motivated the block-aware reserve evaluated below, with
state-lifetime tests and retained-input price comparisons. A global multiplier
or the earlier cold panel's reserve alone is not evidence of a fix. Other
container work remains open; this finite panel establishes neither a global
30M bound nor historical transaction rejection rates.

## Validated block-cache supplement

Successor model
`022232270da81c96dd9d7a12899d52ac011f46144d7f90d778a585aaeca5b7bd`
adds `account_cache_work_units` and `account_cache_scaled` independently of the
existing Account-journal contribution. All inherited prices and the existing
discount remain unchanged.

For finalized transaction account keys `J_t`, each committed/attempted block
prefix maintains its own union `S_t` and first-four-network-byte histogram:

```text
N_t = |S_t|; M_t = largest prefix group in S_t
C_t = ceil((N_t + 4*M_t) / 5)
B_t = 2*|J_t| + persistent_storage_materialization_count_upper_t
U_cache = sum_t(C_t * B_t)
cache_scaled = 128 * 10^18 * U_cache
```

The two potential account visits cover basic loading and a possible commit;
the latter deliberately includes untouched accounts. Storage materialization
adds potential owner-cache visits. Across transactions, a repeated key adds
query budget again while its union membership stays unique. Warm repetitions
within one Journal do not all become DB-cache queries. Rejected candidates
affect only the attempted prefix, and a fresh block resets both populations.
This is an empirical observed-population proxy, not exact DB-cache telemetry
or a universal lookup-cost bound.

The same four retained inputs now produce the following real quotations. No
canonical inputs or original SP1 observations are replaced or rerun.

| Input | Actual PGU | Original af30 node PGU | New node PGU | New full proposal PGU |
| --- | ---: | ---: | ---: | ---: |
| Cold reference | 302,539,274 | 3,780,228,313 | 4,743,492,057 | 4,920,918,088 |
| Cold successor | 946,126,714 | 3,790,845,145 | 6,825,099,481 | 7,002,537,800 |
| Warm reference | 338,631,603 | 1,354,594,058 | 17,441,782,794 | 17,606,176,969 |
| Warm successor | 5,027,210,167 | 1,354,594,058 | 17,441,782,794 | 17,606,189,513 |

The demonstrated warm deficit is covered by the literal node estimate, about
3.469x actual. The deliberately constructed warm reference is also charged
conservatively, about 51.507x actual. Neither is treated as a normal historical
transaction sample, and the large reference overquote is retained.

Across 78 real requotes, all non-cache pricing components, selectors and public
outputs remain equal to their actual af30 baselines. Total changes are exactly
the new cache contribution at 128 PGU/unit, with both ceiling paths and zero
proposal-once contribution checked. All 25 measured controlled inputs remain
covered by full proposal estimates. The same four positive literal-node gaps
remain, the largest 153,352,535 PGU; none meets the >2B priority screen. Three
controlled inputs remain explicitly unmeasured.

The separate historical cohort contains 50 proposals / 9,600 blocks:

| Historical price metric | af30 | New model | Change |
| --- | ---: | ---: | ---: |
| Sum of integer block-node PGU | 92,823,492,832 | 92,857,351,648 | +33,858,816 (+0.03648%) |
| Full proposal PGU sum | 159,967,003,341 | 160,000,862,669 | +33,859,328 (+0.02117%) |

All 50 proposals and 9,600 blocks change. The largest proposal-node uplift is
0.07178%; the largest full-proposal uplift is 0.02901%. The largest absolute
block addition is 48,128 PGU, while the largest relative block uplift is
0.11127%. Median and p95 block additions are both 3,328 PGU. The extra 512 PGU
in full versus node totals belongs to attempted work rather than committed
work; those prefixes remain distinct.

These historical rows preserve the older `416f` / SDK6.1 provenance and have no
numeric actual PGU attached in this sensitivity cohort. They establish the
small price change, not current `b76` accuracy certification or a newly measured
natural-workload error ratio. The two opened warm cases are now calibration
and stress evidence, not untouched holdouts.

Independent source, behavior and numeric gates pass: 266 EVM tests and 59
generator tests, seven independently authored behavior cases plus two focused
source tests, 35 trace and 132 CLI tests, and five scoped host clippy targets.
The independent requote check validates all 78 rows and 19,256 route-component
sums. The full Cargo workspace/all-feature and guest-build checks are not
claimed; an unchanged upstream test triggers one strict-clippy style warning.
New quote executable `9ba9b8f5508483b0da990fd1b308ec818f76fc6a00522eb48bd2bfe78eaee1bc`
retains its actual dbe3563-dirty / Reth c164851-dirty build identity. A later
change only to a test golden is separately checked as production-equivalent.
Requoting uses native GuestInput replay, without new fixture construction,
SP1 sampling, SDK/guest/observer builds or changing the original observations.

Suspended preexecution/direct DB accesses are not claimed to enumerate the
complete cache. Other block-long containers and wider compositions remain
follow-ups. Unzen stays the default; this supplement does not activate a new
mainnet admission rule.
