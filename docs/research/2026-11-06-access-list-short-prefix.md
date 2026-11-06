# Access-list construction with short key comparisons

The raw-low32 supplement does not cover every expensive warm-set construction.
A new single-block input using 14,782,761 charged EVM gas measures 1,322,938,598
SP1 PGU against 142,698,613 literal node PGU and 297,640,444 complete-proposal
PGU. The respective deficits are 1,180,239,985 and 1,025,298,154 PGU. This measured
single-transaction example is below the >2B-PGU/<=30M-gas priority threshold;
it does not settle whether a full block containing multiple such transactions
crosses that threshold. No price change follows from this panel alone.

## Mechanism

The [preceding construction correction](2026-11-06-access-list-construction.md)
prices pairs of declared keys sharing numeric low32. Its demonstrated long-prefix
family has equal first28 bytes in the guest's little-endian U256 comparison.
The new family instead has little-endian limbs L0=L1=i and L2=L3=chosenG, with
i=1..K. If this new execution's own G0 equals chosenG, both inner hash folds are
zero and the remaining two identical terms cancel. Hash body is zero and final
hash is fold(4,G0), independently of the per-map seed and unknown G1.

Every numeric low32 is distinct, so the model389 construction-pair selector is
zero. Different keys are distinguished within their first four compared bytes.
The newly tested work is candidate dispatch, probing and short equality checks
during set insertion. This is the generic warm-set hash, separate from the
journal's fixed32 hash and MPT hashing. It is conditional source algebra, not a
claim that arbitrary low32-distinct keys concentrate.

Both inputs fixed chosenG=dbfc3e5fed9bc5e5 before either measurement. A different
own G is retained as an unrealized control; there was no seed search or retry.

## Measured panel

| Input | Block EVM gas | Actual SP1 PGU | Literal node PGU | Complete proposal PGU | Own-input pressure |
| --- | ---: | ---: | ---: | ---: | --- |
| K64 | 312,361 | 161,465,174 | 8,655,989 | 163,597,820 | unrealized |
| K7680 | 14,782,761 | 1,322,938,598 | 142,698,613 | 297,640,444 | realized |

K64 has a 152,809,185-PGU positive node deficit, while its proposal estimate
covers actual. K7680 has positive deficits on both surfaces. Startup remains in
actual; neither an incremental denominator nor raw Unzen scores replace the
literal node comparison. Each input contains one Anchor and one type2 user
transaction, empty calldata and the five-byte RETURN program, with no user
persistent storage materialization.

K64's own G0 is d0ad7e1083fde0c9, differing from chosenG. K7680's own G0 matches;
31 linked calls independently qualify owner, map, raw key, caller, stack,
chronology, body0 and final hash8066221259537454999. The computed 7,680-key catalog
is not 7,680 dynamically observed placements. Its optional 400M-instruction
observation is truncated; 7,190,675 equality-candidate hits and 3,792 commits are
global watched-prefix counters, not complete comparison counts. Canonical PGU
execution is complete independently of that observation.

## Reusable tools and verification

[concentrated_short_prefix_keys and the two-stage workflow](../../bin/zkgas/bench/README.md#short-prefix-access-list-construction-candidates)
compose with the existing raw-key producer and workload.py screen/measure tools.
The additive helper, tests and usage were committed at4840d46a. All23 original
complete rows and their requests remain byte-identical. The old long-only reader
is retained; a separately reviewed pure postreader explicitly selects the short
family and preserves the original same-call joins, source identities and cap
exclusions. It does not change the observer or canonical measurement contract.

There were exactly two native constructions, two model389 quotes and two SP1
actuals, each once. Retained-input measure reused the original native and quote
bytes and launched only SP1. Independent gates checked signatures, indexed
transaction roots, complete account/state roots, minimal witness, raw key order,
gas, public output and zero construction-pair selectors before actuals. Independent
source and behavior review included128 exact RV64 unit vectors, six public helper
tests, 16 postreader tests and additional negative cases. Both complete actuals
were independently checked against original raw reports and registers.

The frozen b76 ELF, SDK6.3 and observer are unchanged. Actual PGU is exactly
floor(ExecutionReport.gas*10/191): raw gas3,083,984,839 and25,268,127,222. Peak PSS
is1,209,138,176/4,549,979,136 bytes; guarded wall time9.943/48.608 seconds. All eight
owned SHM objects are directly absent on the original host. No guest, SDK,
observer, host executable or pricing model was rebuilt for this panel. Diagnostic
replay time is not GPU proving time or production-SDK certification.

## Remaining block-level question

Single-transaction K15360 is invalid under the current Osaka2^24 transaction gas
cap. Two separate K7680 transactions fit the gas arithmetic at29,398,167 charged
block gas, with a fresh warm set for each transaction. That is a candidate for a
new finite experiment, not an observed two-transaction result or a valid linear
extrapolation of PGU. The frozen native1a used in this panel rejects access_list
combined with its multi-transaction descriptor; a successor host tool must
validate every included signed list and nonce when enabling that composition.
Pricing and block-level severity remain open until the composition is tested.

The subsequent [two-user experiment](2026-11-06-access-list-packing.md) measures
that composition and records its correction separately from this initial panel.
