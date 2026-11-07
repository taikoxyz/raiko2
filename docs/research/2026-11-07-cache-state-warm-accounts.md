# Warm-first account-cache composition

Model `af30` underprices the sole reference-G successor in this finite panel. One
valid block consumes 29,669,999 EVM gas and measures 5,027,210,167 complete SP1
PGU, against a literal node estimate of 1,354,594,058. The positive difference
3,672,616,109 exceeds the agreed >2B PGU / <=30M gas priority screen. The full
proposal estimate also underprices it. This finding requires a pricing fix;
the measurements below preserve the original model, without a correction claim.

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

The immediate follow-up is a node-observable block-aware reserve, with focused
state-lifetime and accounting tests, requotes of these retained actuals and
historical price sensitivity checks. A global multiplier or the earlier cold
panel's reserve is not evidence of a fix. Other container work remains open;
this finite panel establishes neither a global 30M bound nor historical
transaction rejection rates.
