# Storage-map capacity boundaries

The frozen model covers all four repeated-read probes at the observed storage-map growth
boundaries. No price changes follow from this eight-input panel. Four zero controls retain
positive literal node gaps, at most 119,862,744 PGU; the completed proposal quote covers all
eight complete SP1 executions. These are finite synthetic observations, not a block workload bound.

## Inputs and measurement

Each post-Unzen proposal contains one block, its normal Anchor and one user CALL. The CALL
loads K absent slots in its initially empty storage, with the query last, then reads that
query once plus 64R times. K is 448, 449, 896 or 897; R is 0 or 512. No writes, refunds,
rollback, phase slot or second user are involved. The eight inputs are new canonical inputs;
four exported full-level requests were retained but not executed.

External catalogs have eight nonzero bytes per U256 and pairwise distinct low32 values,
including the fixed query's zero low32. Their deterministic arithmetic recipe does not search
guest seeds, buckets or tags. Smaller catalogs use the same ordered pool prefix, with the
query last. This separates the selected growth tests from the deliberately concentrated
comparison catalogs; it does not establish natural transaction frequency.

The existing producer now accepts keyword-only `key_count=512` and CLI `--key-count`, strictly
bounded to integer 1..1024. Default512 complete rows, requests, IDs, runtime and envelopes
retain their bytes. The independent byte interpreter checks the explicit K against calldata,
instructions, order, cold/warm state, metadata and exact gas. For this family user gas is
`21359 + 2416*K + 6775*R`; add 167,355 Anchor gas for the block. Its 15M per-user fixture
envelope, EIP-7825, 31M header-prefix and 30M charged-block checks remain. This panel does
not pack a full 30M block.

Frozen identities:

| Component | SHA-256 / identity |
| --- | --- |
| Model | `5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11` |
| SP1 ELF | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| Execute-only observer | `dfaee7ac41099df02e8a66d9b15b6c4b53d539d1a7e051a67b7180c64d6a0992` |
| Native constructor | `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec` |
| Quote executable | `1390ee779fd09e03dacc4234624f01828ae93dc8995099f4ec8272f64ee69e0e` |
| SDK / executor / runner | `6.3.0` |
| Matrix JSON | `a062ddd664e8dc77e60fec356b26c4c452c65f0ca5cdbbff090fb5756a7e5b68` |
| Measurement summary JSON | `54a4fd1983c4f1432a0def643ac345d35795ad2a20bad65839bff4e25ee3b4c7` |

The profile is `evm-gas-only`. Actual PGU is the canonical complete
`floor(raw ExecutionReport.gas * 10 / 191)`, independently checked against normalized report
gas. The literal node denominator is `quote.node.sum_of_block_known_pgu`; the completed
proposal denominator is `quote.prove.total_pgu`. Unzen raw difficulty is not a PGU baseline.
The historical fixed ELF and SDK are not a current production-program accuracy certificate.

## Results

All workload columns include the same input's Anchor and execution. The mask is observed at
the first warm query in that input; K alone is not evidence of physical capacity.

| K | R | Block gas | First-warm mask | Node PGU | Proposal PGU | Actual SP1 PGU | Actual minus node |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 448 | 0 | 1,271,082 | 511 | 48,634,257 | 203,690,776 | 168,400,305 | 119,766,048 |
| 448 | 512 | 4,739,882 | 511 | 904,842,810 | 1,059,899,329 | 212,984,259 | -691,858,551 |
| 449 | 0 | 1,273,498 | 1023 | 48,725,833 | 203,782,607 | 168,588,577 | 119,862,744 |
| 449 | 512 | 4,742,298 | 1023 | 904,934,386 | 1,059,991,160 | 206,118,290 | -698,816,096 |
| 896 | 0 | 2,353,450 | 1023 | 147,484,410 | 302,655,616 | 175,737,377 | 28,252,967 |
| 896 | 512 | 5,822,250 | 1023 | 1,708,336,035 | 1,863,507,241 | 214,442,885 | -1,493,893,150 |
| 897 | 0 | 2,355,866 | 2047 | 147,640,497 | 302,811,960 | 176,102,105 | 28,461,608 |
| 897 | 512 | 5,824,666 | 2047 | 1,708,492,122 | 1,863,663,585 | 213,631,794 | -1,494,860,328 |

Within each K, subtracting its own R0 gives actual warm-loop increments of 44,583,954;
37,529,713; 38,705,508; and 37,529,689 PGU. The respective node increments are 856,208,553;
856,208,553; 1,560,851,625; and 1,560,851,625 PGU. The selected repeated reads expose no
positive node deficit.

Adjacent K comparisons retain small zero-control increment gaps: actual minus node increases
by 96,696 PGU at 448→449 and 208,641 PGU at 896→897. At R512 actual PGU decreases across
both boundaries. Different signed inputs have their own hash seeds and layout, so these
differences do not isolate the cost of resizing. Nor are all zero-control node gaps proven
to be startup cost: retain them literally, and do not multiply proposal-once terms per block.
No measured positive gap in this panel exceeds the 2B-within-30M severity screen.

## Qualification and limits

Native authentication verifies the signed input, receipts, gas, roots and all K absent slots
against the primary account's authenticated empty storage root. The absence proof derives
from the canonical empty trie; it does not pad a witness with fabricated nodes. All eight
byte/native/quote/actual identities and the completed canonical GasVM reports passed.

The ELF-bound observer samples the concrete private storage-entry caller and correlates all
ordered cold keys, the first warm query, its map, seed, computed hash and mask. Every run
finishes its diagnostic instruction observation, but every run retains `samples_complete=false`.
All seed windows reach 128; R512 query/header windows reach 960, while R0 records
463/464/911/912 query/header samples respectively. Capped rows are not whole-execution
comparison counts or a snapshot of every table entry. The seed is carried only within the
same qualified map lifetime; it is not recaptured live on every query.

Verification for this checkpoint:

- Managed-Python `test_*catalog*.py`: 23 tests, 22 passed; one optional old Native3 integration
  gate skipped. The eight new inputs separately passed native construction and authentication.
- Independent complete source review, old default-byte comparisons, byte execution, gas and
  boundary observations passed. Eight synthetic reader tests and ten actual-controller
  negative-path tests passed before measurement.
- Independent eight-input readback checked full reports, identities, gas and observed masks.
  The root separately checked all 32 newly owned SDK shared-memory names absent on the
  original host after execution.
- No actual was retried. Each used 6GiB process-tree PSS, 600s, 192MiB outputs, 16MiB logs
  and a 576MiB free-disk floor. Observed maximum PSS was 1.461GiB; longest run was 13.554s.

No guest, SDK or broad Cargo build was needed. No proof was generated and no transaction was
submitted to a network. Reusable commands and interfaces are in the
[bench README](../../bin/zkgas/bench/README.md#explicit-storage-catalog-capacity-probes).
Fixed fixture data belongs in Atlas; executable tooling stays in Raiko.

## Next composition

Test ordinary frame rollback followed by slot reaccess, around the same growth boundaries.
Rollback restores warmth and values while retaining loaded slot entries; do not treat it as
physical same-transaction table clearing. The dirty-key family's phase slot adds one distinct
key and must be included. Four-user fixture packing can exceed the 30M campaign budget at
large cold K; a bounded two-user schedule can exercise the same per-user path without
discarding the case or changing the campaign budget. Authenticate its own gas, envelope,
reachable account set and observed phase before SP1 measurements.

The crafted account-journal correction and future L1 occurrence sampling remain separately
deferred TODOs. This panel neither resolves them nor changes their priority.
