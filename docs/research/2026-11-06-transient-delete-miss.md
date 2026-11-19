# Transient deletion followed by missing-key reads

Model923 covers all three measured canonical inputs in this finite panel. The
qualified concentrated R512 probe uses4,262,919 block EVM gas, with actual
380,063,516 SP1 PGU against literal node2,273,193,897 and complete-proposal
2,428,135,728 PGU. No observed positive node or proposal deficit, >2B/<=30M priority
case or pricing change follows. This closes the selected direct-delete composition,
not arbitrary rollback, table distributions, full30M packing or production parity.

## Why this composition

The [prefix/size controls](2026-11-05-transient-prefix-boundaries.md) exercise a
query inserted last in a live transient map. A missing query after deletion has a
different termination path: deleted buckets may remain before a true empty bucket.
Current revm physically removes transient slots on TSTORE0 and rollback-to-zero;
its outer owner remains. A completely empty inner map returns before hashing, so
this panel deliberately retains one live key in the same owner.

Each input has Anchor plus one signed user with empty primary persistent storage.
The raw catalog contains1024 unique keys, with query last. The existing guarded
kernel verifies query0, seeds every key to1 and verifies query1. It then clears
calldata indices1..1023 in order, checks retained index0 is1 and query is0, and
executes64 missing-query TLOAD/POP sites per R. Final counters,128-byte memory,
query stack and empty terminal stack are checked; transient state resets at TX end.
These guards establish logical state. They do not trace every physical tombstone.

The opt-in keeps the model923 ever-materialized footprint after deletion:

```text
N = 1024, M = 1, C = ceil((N + 7*M)/8) = 129
TLOAD = 4 + 64R
TSTORE = 2N - 1
A = 2N + 3 + 64R
J = 2N - 1
U_transient = C * (A + N + J) = C * (5N + 2 + 64R)
U_combined = 54 + U_transient
journal PGU = 384 * U_combined
```

The54 units are retained Anchor persistent work. Node and completed block pricing
add this journal term once; proposal-once journal work is0. At R0/R512 the combined
units are660,792/4,887,864. The ledger does not reduce its concentration reserve to
the one remaining live slot. This bookkeeping is not a physical probe-count or PGU
bound, and the literal block estimate remains the comparison denominator.

## Measurements and the retained unrealized control

Four distinct canonical inputs completed native construction and model923 quoting.
Three SP1 actuals completed once: the original R0 control, then two distinct own-G
inputs at R0/R512. The original R512 input remains explicitly unmeasured.

| Cohort | R | Block EVM gas | Actual PGU | Node PGU | Complete-proposal PGU | Node/actual |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| reference | 0 | 815,435 | 173,671,510 | 364,215,271 | 519,157,102 | 2.097150x |
| ownseed | 0 | 815,435 | 208,590,196 | 364,215,271 | 519,157,102 | 1.746080x |
| ownseed | 512 | 4,262,919 | 380,063,516 | 2,273,193,897 | 2,428,135,728 | 5.981089x |

The two qualified own-G inputs have an actual whole-input increment of171,473,320
PGU, versus node/proposal increments of1,908,978,626 PGU. These are aggregate input
differences, not isolated lookup, deletion or memcmp coefficients. Conservative
quotes in this deliberately selected case do not certify ordinary admission
usability or select a new limit.

The first reference input's canonical execution is valid, including its observed
populated-map miss. Its chosen G was `0xd2b5ccfd88cdb855`, but its own observed G was
`0xad92a98ac23d1c5d`, with S5859713310049070916 and mask2047. Query hash was
3349236668185826430, bucket1150 and tag23. Under those actual parameters the1024
computed hashes are distinct, with only one query bucket/tag match. Its pressure
hypothesis is correctly marked false; it is neither an execution failure nor a
passed concentrated-pressure test. The unrun reference probe is retained as such.

One bounded new cohort used the observed G without overwriting or rerunning old
inputs. Both new actuals independently confirm their own G, seed, mask2047, query
hash0/bucket0/tag0 and same-owner live-sentinel1 followed by populated miss0 with
items1. All1024 catalog hashes are computed as0 under each own profile. This is
not dynamic observation of all placements. The evidence shows a G difference
between these shapes; it does not establish its cause or future seed stability.

## Tools, checks and limits

The reusable `make_raw_case(..., clear_before_reads=True)` API uses the same
producer/kernel/literal VM; it requires a true boolean, explicit raw query and at
least two query-last keys. DefaultFalse leaves all220 prior complete row/request
byte pairs and36 additional raw defaults unchanged. Usage and the VM result
contract are in [the bench README](../../bin/zkgas/bench/README.md#transient-deletion-before-repeated-reads).
Public tooling is committed at `a4b12aa135233832c823e4cb0694621c576d21a0`.

Twelve family tests passed; the18 related shared tests have16 passes and two
explicit opt-in native skips. Independent uncached execution covers12 selected
K2/16/1024 zero/probe/full cases, extreme queries, exact deletion order, whole-state
cache feedback, gas and rehashed-code/typed/metadata negative controls. Independent
source review and behavioral verification passed before native construction.
All four new native/quote inputs subsequently passed independent user/Anchor
signature recovery, complete4-to-5-account roots, minimal six-node witness, three
code bodies, empty persistent storage, routes, receipts and scaled quote arithmetic.

Actual calldata gives gross user gas626,764+6,775R and floor648,080. The shared
calculated full reference R2121/envelope14,997,539 is sizing metadata only; no full-R
canonical input or actual was generated. Every run uses the EVM-gas-only execution
profile, so the historical Unzen zkGas cap does not filter these samples. Raw Unzen
scores retain their separate unit and are not compared numerically with PGU.

The optional host observer adds items and miss-return register watches to the
retained b76 program without rebuilding it. Same-call owner, SP, control, seed,
mask, query hash and preceding surviving-key success distinguish populated misses
from absent-owner or empty-inner fast returns. Both new instruction observations
complete, while sample retention is incomplete;1/13 sampled miss calls qualify.
Whole-program comparison/probe/tombstone counts are not asserted. Canonical GasVM
workload remains independent of optional diagnostic qualification.

Independent raw checks cover all three actuals, exact SDK6.3 options, program/input
identities, public outputs and `floor(ExecutionReport.gas*10/191)` normalization.
All12 owned SHM objects are directly absent on the original host. Peak actual
process-tree PSS is2,107,963,392 bytes and maximum guard runtime21.450s, within
6GiB/600s; native/quote remain2GiB/180s. No runner, helper, guest, ELF or SDK was
rebuilt. The retained quote binary8f6ed77 still has actual dfa18/dirty=true build
provenance with clean upstream cac269, not the later tool commit.

The frozen finite-panel readback has SHA256
`7ba6c7e2dce6ceb568ef0b668afc6a2bea2bff10c2526cf0d97af4819e182345`;
independent actual verification has SHA256
`620594b3a29ca12f8f694a57b45a07365ea347359b9ba7965b82c18730f864fa`.
Model identity remains
`923c8f916f6e8603677b61adb60d92d2cd4355e02030c8fafa6ec549d10525f8`.
Opened development inputs are not IID/holdout samples, proofs or current-production
certification. Caught rollback and implicit removal remain separate compositions.

The [published Atlas fixture](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-transient-delete-miss-v1)
retains all four native/quote inputs, three actuals and the explicitly unmeasured
probe. Its56 safe regular members contain49 byte-identical originals, four
explicit portable projections and three generated metadata/checksum files.
The386,297-byte compressed asset has SHA256
`cca519a3057b4acae671802ef33dbf3692f4287707160463870578e132df7ba8`.
Real draft downloads passed the existing integrity assertions and literal README
unpack/checksum flow; a fresh published download matched compressed and decoded
hashes. These checks establish data retention, not additional workload measurements
or a claim that the entire remote CI matrix passed.
