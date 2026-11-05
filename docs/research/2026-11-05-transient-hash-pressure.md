# Transient lookup amplification and its pricing correction

A selected transient-storage workload demonstrates a high-priority pricing gap. At
3,860,903 charged block gas, its node quote is321,208,226 PGU and its completed-proposal
quote476,150,057 PGU, while actual SP1 work is3,244,570,251 PGU. The literal node deficit
is2,923,362,025 PGU and the proposal deficit2,768,420,194 PGU. This exceeds the user's
>2B-positive-gap-within30M-gas priority screen without requiring full-block extrapolation.
The result is independently verified. A separate transient-map reserve now covers this
measured gap; the original baseline remains unchanged below.

The initial measurements use frozen model5f. Successor model923 quotes are reported
separately below. These are research observations, with no mainnet deployment or claim
that every normal transaction has this workload ratio.

## Controlled construction

The [ordinary three-point panel](2026-11-05-transient-read-catalogs.md) supplies the same
K512 direct-TSTORE seed and pure-TLOAD loop. Each candidate remains a valid single-user
proposal with empty persistent primary storage, no transient MPT targets, the same
14,995,248 signed envelope and the same gross/charged gas at fixed R. The constructor
requests differ only in calldata keys at a fixed R; derived signatures and block/header
identities in the complete GuestInput naturally differ too.

The existing U256 hash helper prepares two finite catalogs using the observed reference
query's seed, shared word and mask. Both have511 unique candidates plus QUERY last,
exactly eight nonzero calldata bytes per key, and computed bucket860/tag103. Long/high32
candidates share the first28 bytes of the guest's key-comparison order with QUERY.
Short/low32 candidates differ at the first compared byte. A new input is not declared
realized merely because it was generated from the reference seed: its own observed
seed, mask and query hash are checked, then all512 catalog hashes are recomputed.

Both searches completed once within268,435,456-trial/120s per-domain limits, under the
existing2GiB/180s process guard. The reused U256-specific helper has SHA256
`2ed5be1fdf5a4556d9f8f1e54ecabcf221179a0916ef41036292a84aa76634e4`.
Its frozen source and512 independent arithmetic vectors were verified. An older helper's
incompatible CLI failed qualification before any search; no guest/SDK rebuild or actual
measurement retry was used to repair that qualification failure.

## Literal baseline measurements

All estimate and actual columns are PGU; gas includes Anchor. Gaps retain their sign.

| Catalog | R | Charged block gas | Node PGU | Proposal PGU | Actual SP1 PGU | Actual minus node | Actual minus proposal |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| Long/high32 | 0 | 475,075 | 35,425,248 | 190,367,079 | 190,167,021 | 154,741,773 | -200,058 |
| Long/high32 | 512 | 3,860,903 | 321,208,226 | 476,150,057 | 3,244,570,251 | 2,923,362,025 | 2,768,420,194 |
| Short/low32 | 0 | 475,075 | 35,425,248 | 190,367,079 | 170,357,529 | 134,932,281 | -20,009,550 |
| Short/low32 | 512 | 3,860,903 | 321,208,226 | 476,150,057 | 698,710,846 | 377,502,620 | 222,560,789 |
| Short/low32 | 2180 | 15,161,603 | 1,252,235,583 | 1,407,177,413 | 2,417,370,218 | 1,165,134,635 | 1,010,192,805 |

The long R512 actual/node ratio is about10.10x. Its actual is about16.94x the ordinary
R512 actual, despite identical node quotes and EVM gas. The short contrast also underquotes;
its full point is one15M user, not a30M packing result. No hypothetical startup multiplication
or replacement of the literal node denominator is used to assign priority.

Long/high32 fullR2180 was independently native/quote-qualified but deliberately not run
in SP1: R512 already establishes the high-priority gap. Across the ordinary and selected
panels there are nine native/quote-qualified canonical inputs, eight actual executions,
and one explicit skipped actual. Seven node gaps and three proposal gaps remain in that
frozen baseline. The three ordinary actuals are reused references here, not three further
measurements.

## Runtime evidence and cause

Each measured input's own successful inner-map and owner profiles qualify. Recomputing
the selected catalogs under each input's observed seed confirms all512 hashes match its
QUERY bucket/tag. R0 directly records512 query candidate-comparison calls. Later selected
profiles are400M-instruction prefixes with capped register samples; their comparator
counts are not full-program counts. Complete canonical GasVM execution and PGU validation
remain independent of these partial diagnostics. Neither computed hashes nor these query
samples are claimed as a dynamic trace of every table placement or compared byte.

Current generic TLOAD/TSTORE pricing cannot distinguish these layouts. The persistent
storage-journal reserve explicitly excludes transient maps. The actual b76 specialization
uses an outer Address20 map and an inner U256 map; this experiment isolates repeated
inner-map queries, not a52-byte combined key. The source and actual query observations
support lookup/candidate-comparison amplification; no persistent proof padding is involved.

The raw highR512 report has gas61,971,291,810, normalized as `floor(raw_gas*10/191)`.
Its canonical input is
`4cb5e13942437c60ebda78025bd559b8ccb61a2424c5927a71ff55fab57ef502`;
raw report SHA256 is
`75021d5d33265cfaf64a556ce1a55954f98354f615a1de5e92ee9667fb349b20`.
All eight actuals use the original b76/SDK6.3 program and observer context detailed in
the ordinary-panel record, with production SDK parity explicitly unverified.

Independent checks cover complete signatures,4-to-5-account roots, six canonical
account-trie nodes, three unique code bodies, gas floors/receipts, actual identities,
executor options, raw normalization, public outputs and source/input bindings. All32
newly owned shared-memory objects are independently absent on the original host.
Maximum actual process-tree PSS is4,545,139,712 bytes and maximum guard time95.540s,
within6GiB/600s. The all-eight audit SHA256 is
`b6792a07f083ac999069a742769c4c283a04020466481305b798b2e25041cc44`.

## Implemented transient-map reserve

The correction gives transient storage its own per-transaction, per-owner ledger,
reusing the existing conservative formula and384PGU/unit tariff:

```text
C = ceil((N + 7*M) / 8)
U = C * (A + N + J)
```

A counts attempted accesses and J potential store-journal work. For the transient
ledger, N/M use distinct materialized full keys, grouped by the actual low32 bits.
Read misses and absent-zero stores do not pre-occupy that membership. Clearing or reverting
a key preserves its membership, while transaction completion folds the charge and resets
the scope. Same-key guard clear/refill must remain linear: R store1/store0 pairs give
U=4R+1, not quadratic growth. The existing persistent ledger counts materialization
events and is deliberately unchanged; directly copying its event behavior is unsuitable
for this normal transient guard pattern.

The persistent and transient ledgers remain separate in the execution snapshot. Their
checked sum supplies the existing storage-journal tariff exactly once in both node and
prove reports. Proposal-once journal work remains0; persistent/MPT resources, other
numeric tariffs, arithmetic domains and discounts are unchanged. Invalid, incomplete or
overflowing work cannot become a valid partial quote. Feature-off execution collects no
new ledger work. Effective storage owners are preserved through CALL/CALLCODE/DELEGATECALL.

The generated successor identity is
`923c8f916f6e8603677b61adb60d92d2cd4355e02030c8fafa6ec549d10525f8`,
from alethia-reth commit `cac269da858031b1d9f9a22ab5141ea0fde2b5dc`.
The original5f identity is retained in the baseline fixture. The actual-source quote
executable has SHA256
`8f6ed77b1bfe110e0e579107b759e917360d45a77ace84b6a87942bd14a44c28`.
It was built from Raiko commit `dfa18ee28560f3f5347637135d96086762d237c0`
with the reviewed consumer pin/tests present as uncommitted changes; its metadata retains
that dirty state. All ten affected dependency nodes were rebuilt together. No guest,
SDK or SP1 program was rebuilt for this correction.

## Completed successor quotes

Each of the same nine canonical inputs was quoted once with model923. The eight actuals
below are the original complete SP1 measurements, not new measurements. Every nonjournal
object, resource and public output matches the baseline; the entire proposal-once object
is unchanged. Node and prove journal increments are checked independently against384 times
their added work units.

| Catalog | R | Actual SP1 PGU | New node PGU | New proposal PGU |
| --- | ---: | ---: | ---: | ---: |
| Ordinary | 0 | 165,384,479 | 337,808,352 | 492,750,183 |
| Ordinary | 64 | 168,650,021 | 1,178,837,593 | 1,333,779,423 |
| Ordinary | 512 | 191,508,491 | 7,066,042,274 | 7,220,984,105 |
| Long/high32 | 0 | 190,167,021 | 337,808,352 | 492,750,183 |
| Long/high32 | 512 | 3,244,570,251 | 7,066,042,274 | 7,220,984,105 |
| Long/high32 | 2180 | skipped | 28,985,366,847 | 29,140,308,677 |
| Short/low32 | 0 | 170,357,529 | 73,813,728 | 228,755,559 |
| Short/low32 | 512 | 698,710,846 | 1,177,485,986 | 1,332,427,817 |
| Short/low32 | 2180 | 2,417,370,218 | 4,773,043,263 | 4,927,985,093 |

For long R512, A=33,282,N=512,J=512,M=512,C=512 gives17,564,672 additional units,
or6,744,834,048 PGU. The measured node quote is7,066,042,274 PGU, about2.178x actual,
matching the projection. All eight measured proposal quotes now cover their actuals;
seven node quotes cover actual. The remaining short R0 node deficit is96,543,801 PGU,
below the2B priority screen. The skipped long full point remains unmeasured.

The ordinary synthetic high-limb catalog also has M512 and is conservatively priced:
its R512 node quote is about36.9x actual. This constructed overquote remains visible.
It does not demonstrate ordinary-transaction admission compatibility or an arbitrary
30M-gas workload bound. More detailed concentration boundaries remain further work.

Independent source and behavior checks passed:232 actual-source EVM tests, including
eight independently added lifecycle/rollback/owner/transaction/feature-off/failure
regressions;40 generator tests and generated-artifact checks;29 trace tests and114 CLI
tests. These used the cached actual-source host dependency closure. Full Cargo workspace,
all-features and guest/SDK builds were not run, and no current production SDK parity is
claimed. An independent numeric readback verifies all nine new quotes and eight actuals.

## Fixed mainnet sensitivity check

The same fixed50 mainnet proposals from2026-10-27 through2026-10-29 were quoted once:
9,600 blocks and235,572,992 original input bytes. This is a historical regression cohort,
not newly acquired latest data. The baseline is model5f, a prior version of the new
pricing model; it is not Unzen and raw Unzen scores are never treated as PGU.

Only two proposals and two blocks change, each by16,128 PGU in both node and prove
quotes, for32,256 PGU total. The other48 proposals and9,598 blocks are numerically
unchanged. All nonjournal objects and every complete proposal-once object match exactly.
The primary node metric is the literal sum of integer block quotes; the whole-proposal
ceiling of block-local scaled work is retained separately and never substituted for it.
Zero price delta does not establish zero transient execution: these aggregate observations
do not expose opcode-specific TLOAD/TSTORE counts.

The retained actuals use original ELF416f/SP1v6.1, while the synthetic actuals above use
b76/SDK6.3. Historical ratios therefore describe sensitivity to this pricing correction,
not current-program PGU calibration, new actual measurements or production certification.
The roughly2x historical proposal comparison is not materially changed by this reserve.

The [Atlas fixture](https://github.com/taikoxyz/raiko2-atlas/tree/55a8575d0ec376cb9f230c44ff65920e8f6c3d30/fixtures/benchmarks/unzen-transient-read-catalogs-v1)
and [immutable release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-transient-read-catalogs-v1)
retain this frozen5f baseline:924,620 compressed bytes, SHA256
`86dbe6db4532fe08bc01ba2d8199346295a2da253946a139c7474fbdd63d509f`,
125 regular members preserving124 original files plus checksums. Eight path-bearing
outcomes are excluded by original hash with portable resource projections retained.
Real remote download, the existing CI integrity assertions and the README unpack workflow
passed; archive integrity is distinct from successor-model validation.

The separate [successor fixture](https://github.com/taikoxyz/raiko2-atlas/tree/36f11ba990b1a9bdb4a1390d94d18ddcad06c03e/fixtures/benchmarks/unzen-transient-ledger-reprice-v1)
and [immutable repricing release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-transient-ledger-reprice-v1)
retain all59 original model923 quotes, the old50-row quote baseline, both model snapshots
and the two independent numeric readbacks. Original synthetic inputs/actuals and235MB of
historical inputs are referenced through their existing immutable assets. Explicit portable
projections identify their original hashes and removed private fields; raw quotes are not
rewritten. The archive has76 regular members, including69 original files; compressed size
936,358 bytes and SHA256
`b0d6ecfccb732d86f378be3eb8aa4602363e59d5552c9d6728992def5fb1751b`.
Independent source/data review, the existing CI integrity body and literal README unpack
commands passed. The latter two used a real GitHub draft download before publication;
a fresh published download also matches compressed and decoded identities. No new quote,
SP1 execution or guest build was needed to archive the observations.

The high-priority finding remains regression evidence after the measured correction;
it is not counted as an unopened holdout. Outer-map owner interactions, the deferred
account-journal issue, future L1 frequency sampling and a full30M maximum remain separate
work.
