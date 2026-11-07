# Committed storage aggregation: covered inputs, unrealized concentration

Both measured inputs are covered by model `02223227`, but the proposed
full-hash-collapse stress was **not realized**. The successor's own shared hash
parameter changed. This panel therefore does not close the block-long storage
container frontier or justify a new tariff.

## Fixed composition and reusable tools

The [exact-sized update helper](../../bin/zkgas/bench/README.md#exact-sized-committed-updates)
was committed in `858c5f8c`. It reuses the existing guarded storage loop and
ProgramRequest schema. Each input has one block, Anchor plus 64 ordinary CALL
users, one predeployed recipient and 4,097 initially nonzero storage slots.
Phase uses raw slot 0, starts 1 and ends 4,097. Each user changes its next 64 distinct
body slots from 1 to 2 and commits the next ordinal phase. User kernels have no
calldata, access list, authorization, CREATE, refund or restore-to-original branch.
The initialized prestate is a fixture precondition; the target block's gas does
not include the cost of first setting up that state on a chain.

The compact raw body key is
`k(i)=(i<<48)|(L1<<64)|(L3<<192)`, with L2=0 and i=1..4096. Reference fixes
L1=L3=0. Before construction, the protocol allowed at most one same-shape
successor whose L3 comes from the earliest independently qualified reference
TransitionAccount.update body-key row. No third input, seed/count sweep or
measurement retry is allowed. The successor selects L3=`0x910c11f2a55b1e09`;
that is a reference parameter, not an assertion about the new input's seed.

Actual execution confirms308-byte runtime, 363,551 gas per user, 364,551 transaction
envelope, 167,355 Anchor gas and **23,434,619 charged block gas** in each input.
The header gas limit is 31M; the severity screen uses <=30M actual charged gas.
The native source's existing `prestate_contracts.initial_storage` path supports
this shape; the older eight-user `storage_state` path is not used.

## Complete workload and literal estimates

| Input | Actual SP1 PGU | Literal node PGU | Full proposal PGU | Actual minus node | Actual minus full |
| --- | ---: | ---: | ---: | ---: | ---: |
| Reference (L3=0) | 377,600,454 | 994,000,808 | 1,315,902,014 | -616,400,354 | -938,301,560 |
| Sole reference-G successor | 377,807,746 | 994,000,808 | 1,315,402,998 | -616,193,062 | -937,595,252 |

Actual is recomputed as `raw execution gas * 10 // 191`: raw gas is
7,212,168,688 /7,216,127,957. Node is the literal sum of individually rounded
block-known PGUs; here there is one block. Full proposal is priced separately.
No startup is subtracted and raw Unzen scores are not treated as PGU. Neither
input has a positive whole-input deficit, including the >2B priority screen.

All 65 signed transactions, transaction root and complete blob data sources are
actually identical across the pair. This was checked afresh, not presumed.
The prestate runtime differs at its three PUSH32 masks; concrete storage keys,
authenticated storage/world roots, witness and canonical inputs change.
Node quote objects are equal. Full decreases 499,016 PGU: block MPT decreases
460,016 and proposal-once parent MPT decreases 39,000; startup and other once
contributions are equal. A paired price/workload delta is not a whole-input
underpricing result or an isolated map coefficient.

## What the local observations establish

The separate bounded observer plan captures the specific
TransitionAccount.update -> StorageSlot entry caller, its actual raw U256 key,
own seed/shared word and full hash, and the parent return. Four watches retain
64/960/960/64 first samples: 2,048 total, within the frozen SDK's 1,024-per-watch and
2,048-total limits. The 400M instruction prefix and execution guards are unchanged.

Independent reconstruction closes 63 local chains /62 body keys per input.
One phase 0 row is separate from the body. The final capped parent interval is
excluded even though other records may exist. All joins verify the fixed
32/10 instruction distances, instruction effects, RA/SP/map/key/result,
chronology, raw-limb order and full-hash arithmetic. They do not infer owner
address or transaction identity from a pointer.

| Local parameter | Reference | Successor |
| --- | --- | --- |
| Own shared G | `0x910c11f2a55b1e09` | `0x8632ac5b84fd984b` |
| Body-key L3 | `0x0000000000000000` | `0x910c11f2a55b1e09` |
| Observed body full hashes | 62 distinct, nonzero | 62 distinct, nonzero |

For this ELF's fixed32 specialization,
`H=F(F(L0 xor S,L1 xor G) xor L2,L3 xor G)`, where
`F(x,y)=low64(x*y) xor high64(x*y)`. L3=the **same call's** G implies H0, but
the new input's G differs from the selected reference G. The conditional algebra
is valid; the intended dynamic condition is absent. Both valid executions and
their PGUs remain usable even though the concentration attempt is unrealized.

The 62 observed hashes are not 4,096 observed placements. Final table population,
whole comparison count, owner/TX identity and whole-input pressure remain
UNKNOWN. Canonical instruction counts 325,096,244 /325,377,649 are complete;
the optional observations are complete with capped samples. These distinctions
do not authorize changing the seed or rerunning the panel.

## Verification, identities and remaining work

Independent checks recover all65 signatures/nonces and transaction roots,
execute the actual runtime bytes, rebuild both 4,097-slot storage roots and full
five-to-six-account worlds, authenticate minimal witness nodes/code, and verify
receipts/gas/routes/refunds, Kona, current model components and public output.
Reference/successor witness node counts are 5,620/5,600 with three code bodies each.
Source/behavior, canonical data, actual SDK/options/resource and raw-local
diagnostic gates are separate and pass. A private comparison oracle initially
assumed proposal-once objects equal; that assumption was corrected to retain
the real parent-MPT difference, without changing product data or rerunning stages.

Each input has exactly one native construction, one quote and one SP1 execution.
SP1 takes 21.02/21.18 seconds with peak descendant PSS about 2.37 GB each, below 6 GiB/600 s.
Both producers and all eight receipt-owned SHM objects are independently absent
on the original host. There is no guest/ELF/SDK/observer rebuild or generated
proof/GPU-timing claim.

| Artifact | Reference SHA256 | Successor SHA256 |
| --- | --- | --- |
| Canonical input | `4754ae4af13d9ea48a8a544748fb93f08b141bd6b70e6669127c4fe0f5942406` | `05a165c5093e2bed5f7d09b9949767d1585a9ff196500efbc3c9effb27788aec` |
| Raw SP1 report | `a54f5fe1edb4c9f56aecbce6296a83801bf1cc9f8458fd7c4353329aa7c7f7f0` | `1b7b4bc65283bc801f4c72ff3c9be3acfa88d3aa242268424499dd039fae5a7c` |

The frozen experimental ELF is b76baf2e with SDK6.3, not current production SDK
parity. Native 9b9e605a retains its actual 350f3dd5-dirty / E3 lower-source identity;
quote 9ba9b8f5 retains dbe3563-dirty / Reth c164851-dirty. Later reviewed commits
and metadata-only pin synchronization do not relabel those binaries. The model
is `022232270da81c96dd9d7a12899d52ac011f46144d7f90d778a585aaeca5b7bd`.
These opened inputs are finite stress evidence, not untouched holdouts, natural
transaction precision, a global workload bound or a production activation.

## Why the shared parameter can change

Static inspection of this exact ELF and its declared matching sources, followed
by independent machine-byte checks, identifies the global seed initializer. It
mixes the first successful initialization's stack address, fixed function/global
addresses and the returned address of a one-byte Rust heap allocation, then
caches six words. The allocator is TLSF. The allocated byte's value, total input
length, instruction counter and SDK getrandom are not direct inputs to this
machine path. A changed G therefore requires changed initializer pointer
context under the ordinary initialization assumptions; the precise earlier
allocation responsible remains UNKNOWN. Equal G would not prove equal context.

The wire deserializes witnesses before the shared proposal node pool. Within
the first witness, state indices and code buffers precede the empty accounts
map; that map still checks/initializes the global seed before its zero-length
branch. Initialization can happen earlier. The raw input is read into a reserved
region, so a shorter bincode does not itself establish a different TLSF return.
Independent binary-pattern readback confirms code-buffer order [308,1,6656]
versus [1,6656,308], alongside the differing witness-index counts. These are
confounders, not an observed allocation trace or the isolated cause of changed G.
No actual first-init stack/heap pointer was captured by this panel.

This motivates a separately scoped host fixture extension: a real Anchor-only
first block followed by the storage workload in a second block. An equivalent
first-witness allocation sequence might initialize G before variable later work;
that is a new hypothesis requiring actual binary-shape and fresh own-G checks.
The first block keeps its required minimal witness and contributes all of its
node/full/actual cost. No unused padding, seed override or guest change is
proposed. The separately implemented
[prefix helper and counts_by_block interface](../../bin/zkgas/bench/README.md#genuine-empty-user-prefix-blocks)
retain real Anchor/root/nonce chaining, per-block minimal read replay for initialized
prestate, proposal-wide read unions and summed per-block proof-node counts.
Focused host tests pass (22 units /3 integrations), as do strict fixture lib/bin
clippy and independent source/behavior checks: five new tiny compositions,23
recovered signatures across13 blocks, seven old JSON/bin/native byte-compatible
cases and34 invalid requests without final artifacts. A style-only lint correction
produces an identical native executable; no guest/SDK/ELF/observer build occurred.
Independent native/typed engineering checks used the tool sandbox namespace;
these are canonical behavior checks, not original-host PGU/SHM measurements.
No formal two-block workload has been measured by this milestone. This closed
pair is not rerun or adapted a third time.

No price change follows. Short-prefix layouts and many smaller committed
transactions remain separate coverage questions: transaction-local reservations
cannot automatically certify a block-long map.

The [data-only immutable fixture](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-transition-storage-updates-v1)
is published at metadata commit f58e6b82088b2d2a88a969f3dbf6f9dac787c768:
60 regular data members (35 exact originals,21 documented JSON-pointer deletion
projections,four generated indexes),2,258,353 compressed bytes,SHA256
`01b430c5736cfdc7480be9becbbb29f260372a07545c4ea2801994e5e7c5dace`.
Independent real remote downloads run both the literal README and existing CI
integrity assertions successfully. No workloads run during packaging; all original
reports and the unrealized concentration remain explicit. It includes no runners,
source snapshots, ELF/SDK/binaries or private path mapping and does not rewrite
previous fixture releases. New-head Atlas CI remains a separate current check.
