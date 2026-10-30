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
