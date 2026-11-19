# Access-list preexecution workload

Continue the search for `actual SP1 PGU / new node-known zkGas` using canonical
post-Unzen proposals, honest/minimal witnesses and valid transactions inside the
existing EVM gas envelope. Complete quotes and matched increments remain separate
diagnostics. EVM-gas-only saturation stays deferred.

Relevant source: the reviewed Reth
[`transact_raw` collector](https://github.com/taikoxyz/alethia-reth/blob/b7060a1ca1a94c22acb786c7d9a54d45d6351dbe/crates/evm/src/alloy.rs)
and the independently recorded access-list resources in
[`CandidateTransactions`](../../crates/zkgas-trace/src/reconstruct.rs).

## Source findings and experiment choice

The frozen b76baf2e program / SDK6.3 workload lock selects REVM handler/context
43.0.3 and context-interface43.0.2. Signed access lists retain raw vectors through
TxEnv conversion. Preexecution iterates every address item and every storage key,
merging addresses and inserting keys into warm sets. Repetition is legal and is
charged intrinsic gas per raw item/key; warming alone does not load account or
storage values from the database.

The new node collector also traverses each item/key, then deduplicates resources.
Model4c0306 charges 256 PGU per signed-envelope byte, 12,162 per unique account and
9,152 per unique storage access. No access-list-specific raw-count tariff exists.
The exact quote ledger independently records declared access-list resources in
candidate scope, so unused warm entries are already covered in resource ownership.
These facts identify a measurement surface, not an established pricing deficit.

Reuse the existing feature-gated constructor and guarded sampler. Extend the
constructor with a typed optional access-list descriptor rather than a second
builder or arbitrary envelope rewriting. Generate ordinary signed EIP-1559 inputs;
allow composition with the existing authorization option for future consumer tests.
Omission preserves old canonical bytes and report shape. Production GuestInput,
guest sources, EVM gas limits and tariff coefficients remain unchanged.

## Fixed 23-input cohort

Use one five-byte `60006000f3` RETURN program with empty calldata. PROGRAM is the
existing target account. Six descriptor patterns distinguish raw counts from
unique resources:

- repeated PROGRAM-address items with no keys;
- distinct address items, starting at PROGRAM and then deterministic absent addresses;
- one PROGRAM item containing a repeated storage key;
- one PROGRAM item containing distinct keys;
- repeated PROGRAM items, each containing the same key;
- the distinct-key list in reverse order.

Use positive typed counts1..8192. This exceeds the maximum payable raw count at
15M transaction gas even for the cheapest 1,900-gas storage-key increment; a
descriptor bound must not artificially constrain legal search inputs. Empty lists
are represented by the existing omitted option. Oversized intrinsic gas remains a
normal native rejection and never increases the transaction limit.

The selected plans are:

1. Empty-list one-block reference.
2. One shared one-address baseline; repeated and distinct address lists at
   K=16/256/4096 (seven address plans including the shared baseline).
3. One shared one-key baseline; repeated and distinct keys at K=16/256/4096
   (seven storage plans including the shared baseline).
4. Repeated address-plus-key items at K=16/256/2048 (three plans).
5. One item with K=2048 repeated keys, paired with the previous grouping boundary.
6. K=4096 distinct keys in reverse order, paired with the forward-order case.
7. Three-block empty-list, repeated-address4096 and repeated-key4096 proposals.

Count-one patterns have identical canonical lists and share one measurement per
shape. Multi-block comparisons use the three-block reference; extra Anchor/block
work must not be labelled authorization- or access-list-only work. All plans keep
the same runtime and require root RETURN success in every block.

## Distribution boundary not established by this matrix

Preserved build records colocated with the exact b76baf2e ELF identify guest
`alloy-primitives` features `std,map-foldhash` without `rand` or `map-hashbrown`.
The address map and storage-key set use foldhash0.2 fast RandomState with the
standard collection backend. Its seeds mix execution addresses and evolving
thread-local state; zkVM excludes the clock branch. This is not a universal
zero/fixed seed, and actual seed values were not extracted. Native Linux seeding
and allocation must not be substituted for guest behavior.

The matched SP1 sysroot identifies hashbrown0.16.1, separately from Cargo's
hashbrown dependency. RISC-V group probing scans eight control bytes, uses low
hash bits for the initial bucket and high seven bits for fingerprints, and changes
the bucket mask when resizing. Concentrated distinct keys could therefore do more
work than sequential or reversed keys; duplicate identical keys test another path.
The 23-input matrix does not establish coverage of deliberately concentrated hash
distributions. That requires a separately reproducible guest-bound input and actual
measurement; source inspection alone is not a demonstrated underpricing case.

## Construction and qualification

Keep raw address/key multiplicity and order in signed RLP, manifest blobs and
GuestInput. Report requested pattern, per-block raw address/key counts and unique
address/(address,key) counts independently of quote counters. Validate typed bounds,
canonical type2/type4 encoding/signatures, exact included transaction identity,
successful execution and declared counts before publication. Do not preload
synthetic accounts/storage just to alter resource reports or witness size.

Focused regression tests cover malformed descriptors, old-byte compatibility,
duplicate preservation, unique controls, grouping/order, multi-block operation,
authorization composition, and EVM intrinsic-budget rejection. Independent review
and native verification precede actual sampling. The same reviewed experimental
ELF is checked against its original immutable source manifest; host-only producer
changes do not justify a guest rebuild.

Freeze new native/quote/model/program identities and a fresh corpus before SP1.
Use the existing sequential 6GiB-PSS/time/output/log/resource-receipt guards. Retain
all plans, native rejects, actuals, literal rankings, full quotes and matched
increments. A tariff change requires actual evidence. Tools and usage stay in
Raiko; fixed observations belong in Atlas after independent raw/archive checks.

## Following priority

Source inspection identifies top-level creation setup as the next separate probe:
it has no preceding CREATE opcode/pending ConfirmedSpawn, while legacy initcode
analysis runs on each creation, including unreachable tail bytes. Existing
transaction, payload and resource terms may cover it; this requires measurements.
Ordinary outer signature recovery and calldata scans already have fixed and byte
tariffs, so their empirical slope controls follow those higher-priority surfaces.

## Completed parent-model observations

All 23 plans completed actual execution on the same b76baf2e ELF / SDK6.3, without
a guest rebuild or changed tariff. Independent qualification decoded the real Kona
blob and signed RLP, recovered signatures, checked declared vectors and 41 trie
states, and confirmed that witnesses do not preload declared addresses/storage.
The final raw audit checked all 23 input/public-output/normalization/tool joins and
all 47 comparisons. Peak process-tree PSS was 1,677,324,288 bytes; all 92 exact
owned resources were absent in the host namespace afterward.

There are three complete-quote deficits under parent model4c0306:

| Shape | Raw items | Actual proposal PGU | Node-known PGU | Complete quote PGU | Full deficit PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| Repeated address, one block | 4,096 | 185,274,324 | 30,235,396 | 185,177,226 | 97,098 |
| Repeated address+key items, one block | 256 | 164,878,063 | 9,796,804 | 164,738,634 | 139,429 |
| Repeated address+key items, one block | 2,048 | 191,970,262 | 35,487,428 | 190,429,258 | 1,541,004 |

The strongest isolation is grouping the same 2,048 copies of one storage key as
2,048 separate address items instead of one item. Unique resources and key count
stay fixed. The 2,047 extra raw address items add 17,391,154 actual PGU, while node
and complete quotes add only 12,051,200: 5,339,954 PGU of increment is uncovered.
All three grouping comparisons have the same direction. Signed-byte pricing
already covers much of the work; it does not fully cover the per-item wrapper.

The three-block empty-list reference measures 166,495,372 actual /168,845,764
complete PGU. Repeated addresses4096 measure 238,385,892 /241,201,348; repeated
keys4096 measure 246,872,119 /272,708,356. Both increments are covered against
their own three-block reference. Forward/reversed unique4096 keys differ by only
1,502 actual PGU with identical prices and covered complete quotes; retain this
variation without inventing an order selector. Literal actual/node rankings and
all old underquotes remain archived, independently of these diagnostics.

## Additive correction selected from these data

Add one normalized charge per **raw access-list address item**, including repeated
items. Keep key pricing, signed-byte prices, unique-resource costs, authorization
validation and the global discount unchanged. Collection can read this count from
the transaction before warming; no future proposal knowledge is required.

The initial policy requires 25% headroom on positive measured workload increments
when the raw item count increases. With fixed-point scale S and delta denoting a
same-block-count pair, each constraint is:

```text
c >= (5 * delta_actual_PGU / 4 - delta_node_scaled / S) / delta_raw_items
c >= 5 * max(actual_PGU - parent_full_scaled / S, 0) / (4 * raw_items)
```

Consider the 40 baseline/adjacent/grouping/three-block comparison rows, retaining
those with positive raw-item and actual-work deltas, plus full-deficit constraints
with nonzero raw item counts. The maximum
is exactly `19375485/4094` PGU/item, approximately 4,732.653884. Integer ceiling gives
4,733; conservative upward power-of-two rounding selects **8,192 PGU/item**.
Zero-item deltas, including the small ordering difference, are diagnostics rather
than constraints that an item tariff could distinguish. This is finite empirical
policy, not a universal bound over hash distributions.

The tariff and rule were fixed before prospective holdout execution. Nine new
holdouts use repeated addresses2/3; repeated items32/512/3072; repeated items512
across three blocks; repeated keys3072; distinct addresses512; and reversed keys256.
Two empty-list reference replays identify the one/three-block baselines and are
explicitly known controls. Holdouts do not enter this 23-input tariff derivation.
Actual SP1 execution completed all eleven reserved rows: nine prospective inputs
and two known reference replays. The known controls have exactly the original
canonical input, public output and measured PGU; they are not nine-plus-two new
independent inputs. The combined cohorts contain **34 measurement rows and 32
distinct canonical inputs**, representing 44 block /88 transaction occurrences.

Two further parent complete-quote deficits occur at repeated address+key items512
and3072. The latter measures 207,723,991 actual PGU versus 205,109,322 parent full
PGU. The three-block items512 case has a covered parent full quote but an uncovered
716,532-PGU increment against its own three-block empty reference. This remains
visible rather than being hidden by startup headroom.

## Actual successor native replay

The frozen host quote binary uses reviewed Reth commit
`cd85071904fe38de8bcad58d82082a37b0e4b7af`, model
`3794f020f947fbaff4b26e8128996fcf9dcffb350b4360cc1d3d4114a809605e`.
It repriced all 34 saved measurement rows; these are actual native quote executions
against retained actual SP1 measurements, not arithmetic-only projections or SP1
remeasurements. The experimental guest ELF remains unchanged.

| Input | Actual SP1 PGU | Parent node /full PGU | New node /full PGU |
| --- | ---: | ---: | ---: |
| Repeated address+key items2048, one block | 191,970,262 | 35,487,428 /190,429,258 | 52,264,644 /207,206,474 |
| Reserved items3072, one block | 207,723,991 | 50,167,492 /205,109,322 | 75,333,316 /230,275,146 |
| Reserved items512, three blocks | 189,261,760 | 35,001,893 /190,895,620 | 47,584,805 /203,478,532 |

All five parent full deficits become zero in this finite cohort. Every node and
full increase is exactly 8,192 PGU times the corresponding raw item count. Old
terms, authorization200k pricing, input/public output and raw Unzen scores remain
unchanged; proposal-once access-list pricing is zero. Seventeen prior authorization
inputs with no access list also retain their previous prices. Small zero-item
ordering differences remain diagnostic; no universal distribution bound follows
from these observations.

The frozen quote has SHA-256
`24c48bcd6be2652bc70bcce85eaaccc3216ac6059cfad17614ba566e10805e18`.
Its modified-source build is recorded as dirty with null embedded revision, with
all 481 inventoried source files checked before/after compilation. A subsequent
commit association is supplemental and must not rewrite the original build claim.
All 34 reports passed independent audit, including eight independently guarded
fresh CLI quote replays. Independent checks decoded the signed access lists and
verified all old terms, exact uplifts and required report fields. The source tree
was subsequently associated with commit
`118eeedbd06e74a3bdc57608633caff5970cd961`, with all 481 committed blobs matching
the build inventory. A later documentation clarification does not change that
record. No production ELF alignment, proof generation, GPU throughput or deployment
is claimed by this host-only validation.

## Retained Mainnet compatibility

Actual successor native replay completed all 50 retained Mainnet proposals /
9,600 blocks using their original SDK6.1 / ELF416f actual-work observations,
separately from the controlled b76b /SDK6.3 cohort. Every block has zero committed
and attempted raw access-list items. All prior terms, authorization contributions,
node/full prices, public outputs and raw Unzen scores are unchanged.

The node-known sum remains 78,776,601,035 PGU; complete quote sum remains
145,920,113,551 against 69,980,978,094 retained actual PGU. Weighted full/actual is
2.085139669739924, with no complete underquote. Thus this additive correction does
not change the existing historical roughly2.09x conclusion; it does not establish
zero access-list usage on Mainnet generally. Replay took79.11s and peak process-tree
PSS122,157,056 bytes under the existing guard. No fresh SP1 execution was required.

## Next measurements

Continue top-level creation with empty/short/long initcode, unreachable tails,
STOP/RETURN/REVERT/HALT and repeated creations inside the same block, distinguishing
initcode analysis from executed opcodes, deployed code and MPT changes. This needs
an explicit top-level creation/multiple-user-transaction constructor extension;
current one-user-per-block experiments do not isolate that slope.

Keep hash-distribution stress as an explicit remaining access-list boundary. Use
the actual frozen guest's hasher behavior if developing concentrated distinct-key
inputs; neither native Linux timings nor ordinary ascending/reversed keys close
that question. Ordinary outer signature recovery and calldata-byte slopes follow.
The deferred EVM-gas-only saturation experiment remains outside this round.
