# Transient read catalogs: ordinary distribution

Three new synthetic single-block proposals isolate repeated TLOAD from the store-heavy
[earlier transient panel](2026-11-03-transient-storage.md). The original model5f node quote covers
the largest measured point; the complete-proposal quote covers all three. The two small
points retain literal node deficits. This finite panel changes no tariff and does not
certify a 30M-gas maximum or production SP1 parity.

## Construction

The reusable producer is `bin/zkgas/bench/cases/transient_read_catalogs.py`; usage and its
independent byte oracle are documented in [the bench README](../../bin/zkgas/bench/README.md).
One normal signed user transaction follows Anchor. It starts with empty transient storage and empty persistent storage at the primary,
checks QUERY=0, writes 512 unique calldata keys to transient value1,
checks QUERY=1, then performs 64 pure TLOAD/POP sites per repetition. QUERY is last in
the insertion schedule. Counters, query, protected memory and empty stack are guarded
before STOP. No persistent SLOAD/SSTORE or extra prestate contract is introduced.

The initial catalog is the deterministic mixed recipe, without seed, bucket or tag
screening. Each canonical 32-byte key has eight nonzero bytes. The explicit catalog
file uses the existing external-data interface so its ownzero and probes have consistent
IDs; this does not turn the recipe into a hash-selected sample.

All three share the maximal-full signed envelope14,995,248. User gross gas is
`21484 + 397K + 6775R`; charged gas is `max(gross, 21000 + 560K)`, with no refund.
The R0 calldata floor is active. Full R2180 is only an envelope reference in this panel,
not an executed full sample. A single user under the fixture's15M transaction cap is
not a 30M full-block packing certificate.

## Literal results

All values below are PGU except the explicit EVM gas columns. Node means the actual
node-known quote, without substituting the complete-proposal estimate or an increment.
Gap is actual minus estimate, retaining its sign.

| R | TLOAD | User gross gas | User charged gas | Block charged gas | Node PGU | Proposal PGU | Actual SP1 PGU | Node gap | Proposal gap |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 0 | 2 | 224,748 | 307,720 | 475,075 | 35,425,248 | 190,367,079 | 165,384,479 | 129,959,231 | -24,982,600 |
| 64 | 4,098 | 658,348 | 658,348 | 825,703 | 71,148,121 | 226,089,951 | 168,650,021 | 97,501,900 | -57,439,930 |
| 512 | 32,770 | 3,693,548 | 3,693,548 | 3,860,903 | 321,208,226 | 476,150,057 | 191,508,491 | -129,699,735 | -284,641,566 |

Each point also performs 512 TSTOREs. At R512, node/actual is about1.677x and
proposal/actual about2.486x. These are synthetic workload ratios, not natural
transaction admission evidence. Neither positive node gap reaches the user's
>2B-PGU-at-<=30M-gas priority screen. Full-proposal coverage does not erase either
literal node deficit. Proposal-side terms are not multiplied once per block.

## Runtime qualification and limits

Every canonical input completed once with the frozen b76 ELF and SDK6.3 GasVM, using
the reviewed streaming observer. There are three fresh native calls, three quotes and
three SP1 executions, with no measurement retry. Actual PGU is `floor(raw_gas*10/191)`.
The raw gas values are3,158,843,565;3,221,215,405;3,657,812,194 in table order.

Independent verification reconstructs each complete4-to-5-account transition, user and
Anchor signatures, receipts, empty primary persistent storage, six selected canonical
account-trie nodes and exactly three unique code bodies. Transient keys are not added
as persistent witness targets. The decoded byte oracle and a separate uncached
interpreter agree on all three complete executions, including the R0 gas floor.

The optional actual-program observer separately qualifies the successful inner-map
query and its owner: one sampled call at R0 and15 bounded calls at each probe. Every
profile has own seed `0x5151e2359b1a6f44`, shared seed `0xd34e0eabce2aabb7`, mask1023,
QUERY hash `0xce9f331c0445a35c`, bucket860 and tag103. This is each input's own observed
profile; equality across these three is an observation, not an assumption for later
inputs. Whole instruction observations complete, but register samples are capped.
Independent computation of all512 catalog hashes agrees with the frozen U256 helper;
only QUERY is dynamically hash-captured. Exactly one computed catalog key matches
QUERY's bucket and tag. No complete table placement or memcmp-byte trace is claimed.

Native and quote guards use2GiB/180s. Actual guards use6GiB/600s,192MiB retained output,
16MiB log and576MiB free-space floor. Actual peak process-tree PSS is at most
1,396,238,336 bytes, with maximum guard runtime11.616s. All12 newly owned shared-memory
objects are independently confirmed absent on the original host. No guest, SDK or
estimator rebuild was needed.

## Identities and evidence

- Model: `5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11`.
- Native executable: `ad0d78dd575228e8685d1f3c0f09f1c4021c76b6d34a9a1341c15e1c4a0e98ec`.
- Quote executable: `1390ee779fd09e03dacc4234624f01828ae93dc8995099f4ec8272f64ee69e0e`.
- SP1 observer executable: `dfaee7ac41099df02e8a66d9b15b6c4b53d539d1a7e051a67b7180c64d6a0992`.
- ELF: `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`.
- Program manifest: `8d5b4a482d26e3bdca94c5e66b9de7b33dc57bcfbaa9b9c24140bf030c1f93a9`.
- Qualification: `90d1abc46d9dc4c1fa005b6a2aa38081a0fcf4b71457664cf1ac549d04ffd3f0`.
- Independent actual audit: `c383e4314498d4c2525469e7dd54a2f5d51d1e13c8d7bb7da2b744b12edc4611`.

Canonical hashes in table order:

1. `6be5ecb0948ee58228990156a9fd2ada81b90cc62a207d6be8909e5fd855878b`.
2. `a5917907554cca5f139ef02ac13b09920b378108cec2cc9ba47b013ca487fb1c`.
3. `b9cfbfb93ed41ec5cdc7d1d0124679fa9b5c83b5fd79de674ded41c6efeeacc7`.

The [Atlas release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-transient-read-catalogs-v1)
now distributes the original inputs, quotes and raw actuals for these three controls
alongside the separately measured selected catalogs. Its metadata distinguishes nine
native/quote-qualified inputs, eight actuals and one explicit skip; these three controls
are not counted again as additional actuals. Private source/input bindings remain local,
and excluded path-bearing outcomes retain original hashes and portable resource projections.
The program manifest is not a full source-to-binary build attestation, and the actual
runner explicitly reports production SDK parity unverified.

The [selected long/short comparison-prefix follow-up](2026-11-05-transient-hash-pressure.md)
subsequently found a high-priority model5f gap and records the measured model923 correction.
Each new input's own seed and mask are checked; this ordinary catalog alone does not
establish safety for concentrated hash buckets. The original values above remain frozen
baseline observations. Account-journal correction and future L1 frequency sampling remain
deferred.
