# Minimal storage proofs expose a node-observable growth gap

Frozen model `c8936124…` covers the complete proposal workload in all26 new
storage-read inputs. Its node-observable estimate does not cover growth in
distinct cold slots. This is a new scalable deficit in the mining surface,
even though the proving surface already accounts for the relevant MPT work.
No price, limit, guest, SDK or proposal-to-block allocation changes in this study.

## Inputs and qualification

The [reusable producer](../../bin/zkgas/bench/cases/storage_access_growth.py)
uses genuine raw logical slots, each initialized to1, in states of8192 or16384
slots. Each one-block proposal contains its ordinary Anchor plus one successful
EIP-1559 user CALL. Warmup checks slot0=1. The fixed64-site loop then repeats
slot0, reads distinct present slots1..N, or reads distinct absent slots1048577
through1048576+N. Counter and checksum checks bind actual returned values.
There are no user writes. Only the exact targets' canonical minimal multiproof
is supplied; unrelated background state is real state, not padded witness data.

The frozen matrix is three modes × two state sizes × N0/64/1024/4096, plus
warm N65536 for each size. All six N0 controls are retained. Independent native
qualification checks concrete ordered keys, values and cold/warm histories,
full prestate storage roots, every target proof, graph reachability of all
provided nodes, completed opcode counts, receipts and node resource counts.
Missing proof/wrong warmup are rejected by native execution. Wrong base/stride
can pass native receipts, checksum and counts; the independent key oracle
rejects them. An opcode count alone cannot qualify these experiments.

Use unchanged native `b0497f45…`, quote `c10eae23…`, sampler `1998fb85…`,
SDK6.3 and source-bound ELFb76. All26 actuals were sampled once after the
input/model freeze. Reported PGU is execute-only `ExecutionReport.gas *10//191`,
not a proof or GPU throughput measurement. The program manifest declares a
source binding, not build attestation or parity with the production program.
The subsequent standard-library-only test hash helper changes no input bytes.

## Results: keep all three quantities

| Case | Node literal sum PGU | Full proposal estimate PGU | Actual proposal SP1 PGU |
| --- | ---: | ---: | ---: |
| Present,16384 slots,N0 |7,685,962|163,040,329|161,319,444|
| Present,16384 slots,N4096 |70,769,586|412,610,793|348,839,656|
| Present,8192 slots,N4096 |70,769,586|363,779,985|316,529,391|
| Absent,16384 slots,N4096 |70,769,586|351,333,161|289,159,868|
| Warm,16384 slots,N65536 |457,949,232|613,303,599|374,277,789|

Full underpricing is0/26; literal node underpricing is24/26, including every
N0 control. Only the two warm N65536 proposals are covered by node pricing.
For present16384/N4096 the literal actual/node ratio is about4.9292 and its
absolute shortfall is278,070,070PGU. The following differences diagnose growth;
they do not replace this literal comparison or allocate actual PGU to blocks.

Among20 high-minus-own-N0 comparisons, all8 warm comparisons are node-covered
and all12 distinct-cold-slot comparisons are node-underpriced. Full increments
cover all20. At N4096:

| Matched series | Actual increase PGU | Node increase PGU, approximate | Actual/node increment |
| --- | ---: | ---: | ---: |
| Present,8192 slots |155,212,835|63,083,623.845|2.4604|
| Present,16384 slots |187,520,212|63,083,623.845|2.9726|
| Absent,8192 slots |101,349,132|63,083,623.845|1.6066|
| Absent,16384 slots |127,840,420|63,083,623.845|2.0265|

The short series also matter: present16384/N64 adds7,182,351 actual PGU against
985,681.623 node PGU, a7.2867 increment ratio. No smaller series is discarded in
favor of the cheaper shared-proof cost per key at N4096. Comparing the same4096
present keys between the two state sizes adds32,310,265 actual PGU and48,830,808
full quoted PGU, with zero node price change. Consequently a single per-key
measurement cannot establish a universal price across state shapes.

The deficit also grows between nonzero workloads: present16384/N1024→4096
adds123,294,475 actual PGU against47,312,717.884 node PGU. It is not merely a
one-time N0→first-read discontinuity.

## Where the gap is recorded

The node hook prices logical EVM-local resources through a path without MPT
metrics. For present16384/N4096, relative to its ownN0:

- Unique-slot resource pricing adds37,486,592PGU (`4096 *9152`).
- Operations add25,597,031.845PGU, including4,755,456 normal-dispatch PGU.
  Dispatch is already included in operations and must not be added twice.
- Together these are the63,083,623.845 node increment. Fixed and transaction
  payload terms do not increase.
- Full pricing additionally records156,003,840PGU of block MPT growth and
  30,483,000PGU of parent-witness MPT growth. Its startup term does not change.
  Its total increment is249,570,463.845PGU and covers the actual187,520,212.

The extra block MPT quote contains hydration30,483,000, materialization120,045,000
and path work5,475,840PGU. These are model terms, not independently isolated
physical costs. Their correspondence with the measured growth supports source
attribution, without claiming exact causal cost from a syscall difference.

`SparseState` hashes the supplied witness and materializes a storage trie on
first account access. Later reads traverse the already materialized trie; this
study does not demonstrate repeated full materialization per SLOAD. More
distinct keys require a larger legitimate target proof and more reachable
nodes to materialize. The existing host MPT collector can observe this work;
the current EVM node hook has neither proof nodes nor path-depth measurements.
The node-visible slot count distinguishes warm repetition from distinct reads,
but does not distinguish these two background state shapes.

## Consequence and next correction

The ordinary-opcode repetition tests previously covered growth in their finite
panel. That conclusion cannot be generalized to cold-state growth. The
remaining node gap is not exclusively a shared startup term. Full-proposal
coverage also cannot certify the current node admission model.

A correction needs an explicit node-visible state-access allowance or a tested
extension that obtains relevant trie work during the block pipeline. Calibrate
against the entire growth series, preserve both present/absent controls, and
test fresh key sets and state shapes before treating a coefficient as validated.
Avoid simply copying proving-only MPT totals into the mining result or hiding
the deficit in an arbitrary startup allocation. Check historical price impact
before selecting a reserve, because both overpricing and cross-surface double
counting would affect normal users. This study selects no new coefficient.

An illustrative price-only sensitivity check on the retained50 historical
quotes finds49,510 block-local unique-slot occurrences in total. A hypothetical
additional100,000PGU per occurrence would increase their aggregate node estimate
by5.3612%. This is not a selected coefficient or sufficient evidence for one:
the cohort is mostly sparse blocks, the hypothetical reserve is not validated
on untouched state shapes, and these historical actuals use a different program.

The existing bounded runner completed all26 actuals under6GiB; peak sampled
process-tree PSS was2,603,974,656bytes and all104 owned shared-memory objects
were verified absent. Retain raw inputs, native/quote/actual reports, source
identities, all controls and resource receipts as immutable fixture data.
