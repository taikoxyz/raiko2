# Two-transaction access-list construction

A complete single-block proposal using29,398,167 charged EVM gas measures
2,484,257,836 SP1 PGU. Model389 quotes207,800,835 literal node PGU and362,742,665
complete-proposal PGU. Actual deficits are2,276,457,001 and2,121,515,171 PGU,
crossing the >2B/<=30M priority screen. This is a measured two-transaction input,
not twice the [preceding single-transaction result](2026-11-06-access-list-short-prefix.md).

## Legal composition and tool support

The block includes one Anchor and two separately signed type2 CALL users, each
with the same ordered7,680-key access list and the five-byte RETURN program.
User nonces are0 and1. Gas receipts are167,355 for Anchor and14,615,406 for each
user, totaling29,398,167. Each user has a15M envelope, below Osaka's2^24 cap.
The fixture retains its existing31M header limit;30M is the independently checked
charged-workload budget for this experiment. Legacy Unzen admission caps do not
screen the `evm-gas-only` experiment.

The host-only [shared CALL/access-list tool](../../bin/zkgas/bench/README.md#multiple-call-users-sharing-an-access-list)
was added at2c88c69c. Every included user is checked against its exact signed
candidate, canonical envelope, signer, expected global nonce and complete raw
list. One report remains per block: raw counts sum every user, unique counts are
true block unions, and nested rows retain each user's counts separately. Here
15,360 raw slots and7,680 block-unique pairs describe different quantities.
Single-user serialization and old canonical bytes are retained. Explicit CALL
descriptors use the existing1e18 sender funding and1Gwei maximum fee policy,
distinct from the old implicit single-user envelope.

Independent engineering checks include13 actual-source unit tests,20 focused
integration tests and15 additional small native cases: successful multi-block
execution, reverts, halts, malformed second lists/nonces/signers, intrinsic gas,
unsupported combinations, exact old serialization and canonical input bytes.
GuestInput, guest, observer and SDK interfaces are unchanged. The new native119c
binary retains its actual4840/dirty build identity;2c88 is a reviewed
source-equivalent commit, not a replacement clean-build identity.

## Complete measured inputs

| Input | Block gas | Actual PGU | Literal node PGU | Complete proposal PGU | First observed map |
| --- | ---: | ---: | ---: | ---: | --- |
| Reference |29,398,167|261,195,526|207,800,835|362,742,665|chosen G does not match|
| One own-G candidate |29,398,167|2,484,257,836|207,800,835|362,742,665|short family realized|

Both were explicitly bounded inputs, each constructed, quoted and measured once.
The reference's first closed call exposed G0=baa3867485e7ef29, differing from the
prior single-input value. That qualified local value supplied one new same-shape
hypothesis. Both original inputs and actuals remain unchanged; no further
candidate or retry is planned. The new input's own local hash context was
independently checked after canonical execution.

The last input's first observed map has31 independently decoded linked calls
with ownG0 matching the chosen value, body0 and final hash16901474841698221222.
Its400M-instruction prefix records6,847,067 equality-candidate hits and3,701
commits. These are global prefix counters. The second transaction's G was not
captured: the local qualification is not a complete two-map placement or
comparison-count certificate. The complete canonical PGU and high-priority
deficit are established independently of that optional diagnostic limitation.

The reference's positive node gap is53,394,691 PGU and its proposal quote covers
actual. The realized candidate's positive gaps are2,276,457,001 node and
2,121,515,171 proposal PGU. Startup is not subtracted, and raw Unzen scores are
not used as PGU. Both quotes use predecessor new model389.

## Identities and verification

Actuals use frozen ELFb76, SDK6.3 and observerdfa, with
`floor(ExecutionReport.gas*10/191)` normalization. Reference raw gas is4,988,834,556;
the candidate raw gas is47,449,324,684. The respective raw report SHA256 values
are25681554fd93dd3b13b1fdcaabbdc3af6c8b3a1120e06f9f780a68aa21481b5c
and411317e8309c5dc348e0cac9c46b6ed947e7a199232fe2df0edd1edb92346a34.

Independent gates verify all three signatures, indexed transaction roots,
complete pre/post account-state roots, minimal six-node/three-code witness,
ordered raw vectors, fee/balance arithmetic, receipts, zero old-prefix selectors,
literal node/proposal totals, input/public output, program/SDK/options and guards.
Approved bincode SHA256 values are
a011223a0fdbde938b4dba0c16ff7cb44a82a97899dc7f352e28c5f15220baa5
and e26a840073636cf800a299504de8ab68faa0796cc89f8c342760305f60903bcd,
both947,144 bytes. All eight owned SHM objects are directly absent on the original
host. Peak PSS is1,744,385,024/4,553,567,232 bytes; guarded wall time13.997/77.920
seconds, within6GiB/600s. These times are diagnostic execution, not GPU proving.

## Correction and retained repricing

The measured growth survives distinct numeric low32 values. The correction
keeps384 PGU per existing same-low32 raw pair and adds96 PGU per same-TX/owner raw
pair with different low32. Raw occurrences, including duplicates, remain explicit;
the two pair counts partition the owner's total pairs. Transactions reset the
counter, node uses committed scopes, prove uses attempted scopes, and proposal-once
contributes zero. This avoids depending on the guest's future hash parameters.

The coefficient is an empirical conservative margin, not a universal workload
bound. Large well-dispersed lists also receive a quadratic reserve; their
overquote remains visible. The new model identity is
`c3065ba83157d1f375d3d34d49f1e9ddfcea5b2bdb7377aae7ad62a3fd28b69d`;
its JSON file SHA256 is
`40d7ca5fa8e2c527561c5cdafcfe57661fbdf2c50c285a161c3a00be742a5dc7`.

The two transaction scopes contribute58,974,720 other pairs. Fresh node quotation
adds5,661,573,120 PGU, giving5,869,373,955 node and6,024,315,785 full PGU: about
2.36x and2.425x the measured candidate. The reference receives the same price,
about22.47x its actual workload, and stays visible as a deliberately
constructed overquote. This is not an ordinary transaction usability certificate.

The existing manifest/quote interface repriced12 retained controlled inputs and
50 historical proposals, preserving their13 original corpus IDs. No constructor
or SP1 run was repeated. Independent readback checks raw signed pair counts,
96-PGU increments, the old384-PGU components, all inherited nested objects and
public output, and both ceiling surfaces. All nine retained controlled actuals
are covered by full-proposal quotations; three unmeasured actuals remain null.
The maximum positive node deficit is152,615,649 PGU for shortK64. No measured
controlled row retains a >2B/<=30M node deficit. This finite panel is not a global
block workload bound.

All50 historical proposals/9,600 blocks have exactly unchanged numeric quotes:
literal summed-block node92,561,066,464 and full159,704,576,845. Their older
416f/SP1v6.1 actuals establish sensitivity, not b76 precision certification.
New quote executable SHA256 is
`a3d6530f9a70e7e9576def83254f004631fb7f45367fb65f70620bf4d8138e02`;
its actual source identity remains Raiko fe717dirty/Reth75ddirty. Later reviewed
commits do not relabel that binary as clean. Independent246-case EVM verification,
31 trace/119 CLI tests and the strengthened47-case compare target passed. Exact
production metadata Clippy passed. Post-pin formatting and locked offline no-deps
metadata passed; workspace Clippy stopped before lint analysis because the exact
Git checkout was unavailable offline. A full locked Cargo/workspace/guest build is
not claimed. Reviewed source-equivalent commits are
[Reth8eddc9b8](https://github.com/taikoxyz/alethia-reth/commit/8eddc9b8fdbb0f569ff4bdc4101f2e47e8cd2730)
and [Raikof832bbbd](https://github.com/taikoxyz/raiko2/commit/f832bbbdd72ac581902c3b1a1af53eb6762f45d9).
The outer warm-owner AddressMap remains a separate unmeasured follow-up.

The retained [Atlas fixture](https://github.com/taikoxyz/raiko2-atlas/tree/c757b8f0645dd58f213cab2390ebc7b16aea26bc/fixtures/benchmarks/unzen-access-list-short-pairs-v1)
and its [published immutable asset](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-access-list-short-pairs-v1)
contain the four new inputs/actuals and all62 original successor quote rows, with
immutable references for the earlier inputs. The existing fixture CI assertions
and literal README checksum/download workflow passed against the actual published
asset. Compressed SHA256 is
`b301edd97fd10e7e0313008bb4bf1b45bec568ed9374326a55b01b98bb87c3d0`;
the decoded tar SHA256 is
`158d3faee2246c77da0cb629d37a672cc3bd809fecb60f15a2c9faf3900257a5`.
