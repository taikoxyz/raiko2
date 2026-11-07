# Same-owner warm density across two transactions

This fixed experiment asks whether a second transaction against the same owner
changes the relationship between complete SP1 work and the current block-observable
estimate. The [single-user density point](2026-11-08-storage-warm-density.md) had only0.900% node headroom;
that result remains a narrow measured margin, not a guarantee for composition.
The new proposal contains two genuine blocks: an Anchor-only prefix and a second
Anchor followed by two user CALLs, schedule[0,2]. It uses the existing structured
factory with an explicit user_count2 option. No new pricing coefficient, guest
schema, model selector or public execution strategy is introduced.

Each user seeds1024 distinct absent storage keys, then performs1536 batches of64
warm reads plus one initial warmup read. That is98,305 warm and99,329 total SLOADs
per user. Both users have identical calldata/code and the same owner; aggregate
counts are2048 cold,196,610 warm and198,658 total reads, with no persistent writes.
The query is the final seeded key. Raw keys follow `(i<<24)|(L1<<64)`, with L2=L3=0.
EVM access warmth is fresh for each transaction. The proposal's backing cache may
reuse entries, while the Journal and its read-frequency pricing remain transaction
local. This tests that composition directly without assuming physical cache reuse
makes the second transaction free or doubles actual proving cost.

## Closed finite-pair results

| Metric | Reference | Sole successor |
|---|---:|---:|
| L1 source |0|THIS-reference selected shared G|
| Complete actual SP1 PGU |418,145,896|11,220,842,425|
| Literal node estimate |11,475,014,309|11,475,014,309|
| Aggregate-rounded node estimate |11,475,014,308|11,475,014,308|
| Full proposal estimate |11,630,463,939|11,630,463,939|
| Actual proposal EVM gas |25,924,303|26,120,911|
| Actual block gas |167,355 /25,756,948|167,355 /25,953,556|
| Node / actual PGU |27.442609x|1.022652x|
| Full / actual PGU |27.814368x|1.036505x|
| Actual minus node |−11,056,868,413|−254,171,884|
| Actual minus full |−11,212,318,043|−409,621,514|

Reference actual PGU is independently derived as `7,986,586,619*10//191`.
Literal node pricing sums the integer ceilings of the two block reports; replacing
it with one ceiling over their scaled aggregate would subtract one PGU. Full pricing
retains every proposal startup/once component. Storage-transition's proposal-once
zero is not an assertion that the entire proposal has no once cost.

The reference quote is27.443x its actual PGU at the node level and27.814x for the
full estimate. This deliberate synthetic reference is strongly overquoted; it is
not a measurement of normal mainnet precision. The successor is covered by both estimates, with only2.265% node headroom.
Its exact ratios are11,475,014,309/11,220,842,425 and
11,630,463,939/11,220,842,425. Actual is `214,318,090,319*10//191`.
This does not replace the previous single-user0.900% margin or establish a
separate warm-read tariff upper bound. No repricing followed this finite pair.

## Input and observation qualification

DATA verified four recovered signatures: two Anchors and two user transactions
with nonces0 and1. A single canonical invocation consumed two real VM results,
one per user, checked their individual gas and read footprints, then summed routes.
The final sender nonce is2. World roots, transaction roots, ordered Kona user lists,
receipts and typed code order were checked against the retained input. Configured
code sets are exactly three in block0 and those three plus one EMPTY code in block1.
The owner starts and ends with empty storage. This does not claim the entire
configured witness is an actual-read-minimal witness.

The reference independent profile sealed255 strict closed generic locals,247
declared-key members and one bounded-observation issue. Selection uses the unique
earliest row over ALL eligible generic locals, not membership or a nonzero-hash
filter. Its start is32,710,472, key0xc9, with shared G13,001,931,078,669,107,839.
This is a nonmember row. A numerical G equal to an older experiment is not evidence
that an older G was borrowed: selection is bound to this reference's raw, input,
independent actual report and independent profile seal. The successor independently matched that fresh process G and all247 declared-key
member locals had full hash zero. Both profiles retained255 strict locals and one
issue; the reference member hashes were nonzero. These local qualifications
do not identify Journal ownership, transaction identity, deepest occupied hit or
whole-map placement. Optional observation and sample completeness are both false;
complete canonical actual PGU is qualified separately from those bounded windows.

## Scope and next conclusion

The recipe source is helper checkpoint3f60193f. Quote401a retains its actual
Raiko200ccc-dirty/Reth5d-dirty provenance; native1368 is historical construction,
state and gas support only. The measured program is b76 with SDK6.3, not a claim
about production SDK6.8. None of these identities is relabeled to the helper commit.
This finite proposal is below30M gas; it does not establish a30M maximum or a
universal PGU bound. The cohort grows from102inputs/9674blocks/49 numeric actuals to104/9678/51.
Node coverage becomes44/51 and full coverage48/51. The existing seven positive
node gaps and three positive full gaps remain unchanged; their largest node
gap is1,597,968,115PGU. No measured point exceeds the strict2B gap priority
threshold at≤30M actual gas. Fifty historical proposals/9600blocks without current
actuals and three controls remain outside accuracy certification. Independent numeric/cohort verification passed (report`c8b25546`, manifest
`0d9fc6e5`), including old input identities and all prior residuals.
A metadata-only root profile preflight rejected a stale reference key identity
before invoking the reader; the retained correction note is not a measurement
retry. Each input used one helper/native/quote/typed/artifacts/canonical/SP1 call,
with two VM executions in its one canonical validation. The earlier warm-tail
Atlas candidate remains local commit2d19 without public upload; local documentation
and helper commits do not imply release or PR sync.
