# Structured Journal warm tail: a closed price-ratio checkpoint

The fixed 4096-key, 512-batch reference/sole-successor pair is closed under unchanged EEA pricing. The successor used **8,396,992,523 actual SP1 PGU** against **10,224,265,060 literal node PGU** at **13,755,636 charged EVM gas**. Its complete proposal quote was 10,380,501,122 PGU. Both inputs are covered by node and full; this panel identifies no new pricing deficit and makes no price change. It does not establish a bound for an arbitrary 30M-gas block.

This is an opened, reference-informed stress test of the margin for many warm accesses after a large same-transaction Journal has been populated. It is not a blind holdout, a natural-traffic accuracy sample, or proof that the final queried key occupies the deepest table position. The large reference overquote is retained alongside the successor result rather than discarded to improve apparent accuracy.

## Fixed workload and the prospective selection rule

Each proposal has a genuine Anchor-only first block, followed by another real Anchor and one ordinary CALL user: `[0,1]` users, three signed transactions in total. The primary code account exists with empty persistent storage; there are **zero preloaded target slots and zero persistent writes**. The kernel reads 4096 distinct absent slots, requires every returned value to be zero, retains the last key as its query, performs one warmup, and then executes 512 batches of 64 warm reads. Thus R means batches, not reads: **4096 cold + 32,769 warm = 36,865 SLOAD events**. Counters, sentinel, zero feedback, query identity and empty-stack STOP are guarded.

Raw keys are `(i << 24) | (L1 << 64)`, for i=1..4096, with L2=L3=0 and no phase slot. They share three leading little-endian bytes. Distinct full U256 keys can share a raw-low32 group; this is not full-key equality. K, R, order and last query were fixed before either actual. In particular, last insertion alone is not a deepest-hit guarantee after table growth.

The reference uses public parameter L1=0. Its prospective selector was the minimum `start_instruction` over **all** independently qualified, strictly closed generic PlainStorage locals. There was no membership, owner, transaction, nonzero-G or desired-numeric-value filter. Missing eligible evidence would stop the successor as UNKNOWN; G=0 would close it as a duplicate. A successor whose fresh process G differed would be UNREALIZED, with no third adaptation or changed window.

The selected reference row was the nonmember key `0xc9`, instruction 33,793,113→33,793,678, with process-shared G=13,001,931,078,669,107,839. This was not relabelled a body insertion. The sole successor used that value as L1 and independently observed the same process G afterwards. No old panel's G was used as qualification of this new input.

## Whole-input numbers and units

| Input | Charged EVM gas | Actual SP1 PGU | Literal node PGU | Full proposal PGU | Actual minus node | Actual minus full |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Reference | 13,362,420 | 270,752,221 | 10,224,265,060 | 10,380,501,122 | -9,953,512,839 | -10,109,748,901 |
| Sole successor | 13,755,636 | 8,396,992,523 | 10,224,265,060 | 10,380,501,122 | -1,827,272,537 | -1,983,508,599 |

Actual PGU is `floor(raw SDK execution gas * 10 / 191)`, using raw gas 5,171,367,428 and 160,382,557,206 respectively. Literal node is the sum of the two block integers, **7,329,931 + 10,216,935,129**. The aggregate scaled ceiling happens to equal that sum here; it is not a substitute for the literal denominator in other inputs. Full proposal is a separate complete quote. Both Anchors, prefix, startup and proposal-once costs remain included; storage/account reserve once-zero does not mean all proposal-once costs are zero. No startup subtraction or raw Unzen-to-PGU substitution is used.

The successor's node/actual ratio is **1.217610** and full/actual is **1.236217**. The reference's node/actual ratio is **37.762442**, with full/actual 38.339487. These are deliberately preserved overquotes, not evidence of natural-traffic precision. The identical quotes reflect conservative population/query-budget proxy reservations under the same pricing terms, not demonstrated recognition of each actual hash seed or probe rank. All existing pricing terms participated; the panel did not fit a tariff to these two opened actuals.

Gas is not equal across the pair. Literal calldata byte counts affect intrinsic gas and the EIP-7623 floor. The user charged 13,087,931 gas in the reference and 13,481,147 in the successor; each is measured independently. Actual Anchor receipts were 167,355 and 107,134 for each proposal. Adding them gives the table's 13,362,420 and 13,755,636, not the earlier conservative two-1M-Anchor source allowance. Input, signatures, roots, configured witness, Kona payload and gas all received fresh per-input validation; no paired-equality assumption replaced those checks.

## What the observation qualifies

Both profiles contain 255 strictly closed generic Plain locals, of which 247 are declared-keyset members. Reference member full hashes are nonzero; all 247 successor member locals have full H=0. Each local was checked against its own caller/RA/SP/map/key pointers, operands, seed, shared word, return and chronology. The capped final interval was excluded conservatively. A member is only a match against the declared keyset, not proof of semantic owner, transaction, Journal map or physical target-body identity.

The frozen default-hasher source connects generic PlainStorage and Journal U256 maps to one process GlobalSeed, distinct from each map's own S. With L1=G and L2=L3=0, the zero-factor fold branch implies Journal H0 independently of S **conditional on that source relation**. This is source-implied algebra. It is not a direct dynamic Journal-H observation, a measured deepest occupied hit, a comparison count, BundleState PGU attribution, final population or proof of all placements/whole-map pressure. Those stronger claims remain UNKNOWN. Observing matching generic G in both executions does not establish allocator stability for arbitrary future inputs.

| Input | Canonical instructions | Optional processed instructions | Observation complete | Samples complete |
| --- | ---: | ---: | --- | --- |
| Reference | 241,298,454 | 241,298,454 | true | false |
| Sole successor | 10,485,919,465 | 400,000,000 | false | false |

Two questions therefore stay separate. The numeric question is whether the retained complete execution costs more than its literal quote; here it does not. The mechanism question is which table and occupied position account for particular instructions. Generic process-G evidence and matching Plain locals support the frozen algebraic hypothesis, but do not answer that second question for the Journal tail. Even the 247 zero-hash locals cannot be expanded into a dynamic statement about every one of the 4096 cold keys or all 32,769 warm accesses. There is no extrapolation from this sampled prefix to whole-map comparisons, and no multiplication of an earlier point's PGU to supply missing observations. This separation allows the whole-input result to remain useful without manufacturing a stronger attribution claim.

The complete canonical execution report supplies actual PGU independently of these optional diagnostic limits. The reference's observation is complete but its retained samples are capped; the successor reaches the fixed 400M prefix. Neither missing samples nor whole-PC counts are used to fill an unobserved Journal warm tail. Plan754/reader70da and its 2048 retained-sample allocation were not widened, shifted or replaced after data.

## Verification correction and provenance

Each point had exactly one formal helper, native construction, original current quote and SP1 execution. There was no measurement retry, third input or seed/count/R/query search. A deterministic Python DATA-oracle correction is recorded separately: the reference's first canonical validation ran its literal VM, then failed an overly narrow code-set assertion. The unchanged input legitimately exports three configured code bodies in block0 and those three plus exactly one EMPTY code in block1, carried from the prior execution's bundle. EMPTY is distinct from the one-byte STOP code.

The correction changed one canonical assertion, accepting those exact blockwise sets while preserving code order, world roots, signatures, query/calldata intent, gas, pricing and Kona checks. It neither removed nor normalized input code. Old failed START/POST/stderr/exit records remain immutable and are not called PASS. Reference literal VM validation count is transparently **two**; the successor used the corrected validator from its first canonical pass and has count **one**. Typed inspection, native, quote and SP1 were not repeated. This was a verifier qualification boundary, not a guest, state-root or pricing-model bug.

The per-input DATA gates also checked all three signatures and indexed transaction roots, fresh fee/nonce/calldata fields, initial/final authenticated world roots and root carry, actual receipts, and the existing Kona/blob payload. The primary's empty storage root establishes absence without adding a separate witness record for every declared raw slot. Empty persistent storage does not mean the execution Journal/cache remains empty. In the recorded source path, this existing code-bearing owner has Loaded/Changed status with `is_storage_known=false`: a cold load still follows `PlainStorage.entry → witness DB.storage → insert0`. The empty storage trie can return zero cheaply; it does not bypass that entry path. Subsequent warm repeats use the Journal occupied entry rather than another DB load. These are source-path distinctions, not dynamic qualification of every map/probe in the observation. The workload reserves remain part of the quote. Conversely, complete configured code/witness sets include code not executed in the Anchor-only block; they are not presented as a proof of globally minimal actual-read witness size. Typed order is checked from the retained binary and is not inferred from human-readable JSON object ordering. No evidence of allocator equivalence is inferred from matching code lengths.

The b76 ELF, dfa observer and SDK6.3 execute-only estimator remain the original artifacts, with no guest/SDK/observer/ELF build. This is not production SDK6.8 parity, GPU proving time or a generated-proof speed result. Guarded execute wall times were 15.60s and 228.97s; sampled process-tree PSS was 1,856,288,768 and 4,553,237,504 bytes. Both ran under the declared 6GiB/1200s guard. These CPU-side resource observations are not hardware-independent performance bounds. Exact owned producer/SHM absence was checked separately on the original host without signals or deletion.

The frozen model ID is `eea6f1da87ee129d2d9f759ae6cef01cb8ac93988376a86efb290398e6db5da6`; quote executable SHA is `401a7e3d7ff6984213b1d6ad9c7dcd308ba6372b461b8ba097c25ededaf4f08d`. Quote401a retains its original Raiko200ccc-dirty/Reth5d-dirty build identity and current EEA pricing scope. Native1368 retains its historical b3f96424-dirty/older lower Shadow identity: constructor, schema, canonical and native execution validation only, not current EEA native pricing parity. Later clean tool commits do not relabel these binaries or reports. The actual and quote results refer to exact retained canonical inputs, rather than reconstructed JSON or re-signed substitutions.

## Retained identities and composition

These two inputs are currently retained locally, **not a published Atlas release**. The following exact hashes and byte lengths identify originals for a later immutable data-only fixture; no private source paths or unpublished download command are required to identify them.

| Role | Original artifact | Bytes | SHA-256 |
| --- | --- | ---: | --- |
| Reference | canonical input.bin | 474671 | `46320c861560614bb11c748b743893f51f41ee672e54eac0ed5b063d81706797` |
| Reference | native.json | 4686 | `e53a28895c99be0388384114bd83e2eea60c56d0cea45867e9e089fbcaeb5bd4` |
| Reference | quote.jsonl | 10391 | `485760e50bd8ad9b8e55877b43bf5d6fc7e5496ddaf29d952e967ddb0e72bd70` |
| Reference | raw SDK sp1.json | 850001 | `5c32ca21877a312eaf1f6c3d24bafc93f3ec89fe06253e652eecd0bf6c9ad135` |
| Successor | canonical input.bin | 474671 | `bfc561b55d92f420c66f2a59b3244c766e3ea59cde2d67fa08f11ab8f7ca9e20` |
| Successor | native.json | 4684 | `e9cdcf59b48a8a1f200ce09b5cdf5a5ae283572c24b4f8cd1f1402c5487bfee8` |
| Successor | quote.jsonl | 10391 | `c323e446ead9040c539f94875e0bd47b3afe8bfa729084d9f0249a8ab1ed8185` |
| Successor | raw SDK sp1.json | 869477 | `9025b954202185fbd733e87afb21df456958f8e6d159657745f026a08abcd64b` |

The reusable public factory is [make_structured_case](../../bin/zkgas/bench/cases/warm_read_catalogs.py); its tiny engineering use below is not either formal input. From the repository root, with a consumer-provided managed `PYTHON_BIN` and Python optimization unset:

```sh
env -u PYTHONOPTIMIZE PYTHONPATH=bin/zkgas/bench/cases "$PYTHON_BIN" -S -B - <<'PY'
from warm_read_catalogs import make_structured_case
row = make_structured_case(raw_key_layout={'L1': 0, 'L3': 0},
                           key_count=3, repetitions=2)
request = row['request']
PY
```

Existing [workload.py](../../bin/zkgas/bench/workload.py) `screen` and `measure` interfaces, described in the [benchmark README](../../bin/zkgas/bench/README.md#explicit-structured-absent-storage-warm-tail), own composition and execution. Consumers supply their own standard artifact/model/resource configuration and retain fresh outputs separately. This record adds no runner, execution schema, private replay command or automatic release dependency. Future consumers must check exact format/backend/model compatibility; immutable historical input identities do not imply compatibility with an arbitrary newer guest.

## Follow-up scope and next boundary

The current storage/account follow-up checkpoint contains **100 unique inputs / 9670 blocks**, with **47 numeric actuals**: node covers40 and full covers44. Seven older positive node gaps and three older positive full gaps remain; the maximum positive node gap is **1,597,968,115 PGU**, with zero above2B. The newly covered warm-tail pair does not erase any residual or justify calling all samples covered. The 50 historical proposals / 9600 blocks and three explicit unmeasured controls lack numeric actuals and are not current precision samples. This scope is storage/account follow-up, not the entire opcode/precompile campaign or a global 30M-gas guarantee; see the [preceding read-cache checkpoint](2026-11-07-prefixed-storage-read-cache.md) and [coverage ledger](zkgas-workload-coverage.md).

The next logical boundary is the warm-heavy ratio and its combination across multiple transactions. It requires a separate prospective source/gas/count/selection specification and gates before any new input. No extra R, user count, seed or observation window is appended to this closed pair, and no new price is inferred from it.
