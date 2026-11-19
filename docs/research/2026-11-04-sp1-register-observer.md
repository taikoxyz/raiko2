# Optional SP1 historical register observer conformance

The host-only observer passed independent source review and bounded behavior qualification on
SDK 6.3.0 / the retained b76 ELF. This is tool conformance: **zero new pricing samples**,
no new workload cohort, model, price, guest, SDK, ELF, pin or canonical input change.
The [warm-read catalog baseline](2026-11-04-warm-read-catalogs.md) remains immutable.

## Interface and execution boundary

The [estimator usage](../../bin/zkgas/sp1-estimator/README.md) documents optional
`--observation-plan plan.json`; [workload usage](../../bin/zkgas/bench/README.md) supplies the
ordinary invocation. The binding [spec](../superpowers/specs/2026-11-04-sp1-register-observer.md)
and [plan](../superpowers/plans/2026-11-04-sp1-register-observer.md) define this increment.
Without a plan the original report shape is preserved; with one it adds `observation`.
The canonical GasVM remains authoritative for gas, instructions and public values.
After GasVM consumes each held immutable raw chunk, public typed `SplicingVM` steps recover
selected registers before/after instructions, then release the borrowed guard.
There is no arbitrary RAM read, native mmap inspection, suspension, private dispatch copy or full record retention.

Plans are bounded to 64 KiB, 64 unique executable PCs, distinct registers 0..31,
0..1024 samples per watch / 2048 total, 1..1,000,000,000 instructions and 4 MiB output.
Unknown/duplicate fields, wrong ELF or invalid domains fail before runner/resource creation.
Counts continue beyond sample caps. Replay `complete:true` differs from `samples_complete:false`:
count-only watches and capped retention can truncate samples while replay reaches halt.
Instruction limits/errors explicitly mark replay incomplete; canonical estimation continues where possible.
Panic handling does not catch abort/OOM. External guards remain necessary.
The 134,217,728-memory-entry threshold, two slots, canonical options and stdin framing are unchanged.

## Initial three diagnostics and one additional boundary check

All four diagnostics passed once, with zero retries, three distinct retained inputs and zero new pricing samples.
Exact original raw gas, normalized PGU, public values, input/ELF, framing and options matched;
only bound build/executable identities, diagnostic time and the optional observation extension differed.
`retained_events_presets` alone was compared as an unordered set, rejecting duplicates.

| Diagnostic | Raw gas | PGU | Observed instructions | Completed chunks | Retained samples | Seconds | Peak tree PSS bytes |
|---|---:|---:|---:|---:|---:|---:|---:|---:|
| Initial ordered zero, default | 3239265915 | 169595074 | n/a | n/a | n/a | 5.529682748019695 | 1221337088 |
| Initial ordered zero, observed | 3239265915 | 169595074 | 146161581 | 1 | 1698 | 10.20060384646058 | 1247909888 |
| Initial ordered full, observed | 6078532964 | 318247799 | 323092511 | 1 | 1924 | 21.119648730382323 | 2391791616 |
| Additional EXP sparse24 full, observed | 15621051982 | 817856124 | 915598098 | 2 | 46 | 48.31620816141367 | 3485472768 |

The default row exports no observation/chunk count. All three observed replays completed;
all have `samples_complete:false`. Selected load PC+4/clock+8, unchanged selected non-destination
registers, counters, caps and encoded-size checks passed. Initial observations were single-chunk.
A separately authorized EXP check then exercised a real successive-chunk transition: its
194,582,938 minimum data-memory entries exceed 134,217,728 + 512, requiring multiple chunks.
Its copied nine-PC plan raises only the instruction limit from 400M to the reviewed 1B maximum.
PC/clock/all-32-register continuity, empty read cursors and final halt are enforced by reviewed
source and exercised at runtime; raw per-chunk boundary/cursor snapshots are **not exported**.
EXP has no 512-key user catalog or EXP-user map/seed attribution; watch counts can include Anchor.

Unchanged external guards were 6 GiB sampled process-tree PSS, 600 seconds, 16 MiB logs,
192 MiB growing output and 576 MiB free disk. All 12 initial plus four additional unique owned
shared-memory names were receipted absent on the actual host, device 27 / original mount namespace;
cleanup reported already removed, with no verifier deletion. Times include extra host replay work,
not prover/GPU time or replacement pricing measurements.

## Conditional warm-map observations for later experiments

Both warm-read replays correlated one chronological candidate map `0x7863af80` with all 512
actual cold/vacant key tuples from the authenticated ordered catalog: first cold global header
hit 15, first warm hit 527. Observed `S=0xff5779f5c8e3fd0c`, `G0=0xefaa3143bec6736f`
loaded through `0x78581850`, first warm `H=0x58d6f28575fdb3bc`, mask 1023 and ctrl `0x78713c30` agreed.
For `F(x,y)=low64(x*y) XOR high64(x*y)`, all selected cold keys satisfied
`A=F(k0 XOR S,k1 XOR G0); H=F(k2 XOR A,k3 XOR G0)`.
This remains a conditional map/key/clock association: no direct account owner, warm lower128
read, whole occupancy or other five shared seed words were observed; hash agreement is no collision-free proof.
Whole-PC counts include Anchor. Observed full-minus-zero entry/seed/shared/header/occupied/memcmp
counts were 129792; vacant/reserve/init deltas were zero. These were not mandatory bounds.
Independent disassembly establishes a 32-byte memcmp here, correcting an older 48-byte claim
without rewriting old reports. The generic tool embeds no ELF PCs or seed constants.
Qualified data can support separately declared custom-key experiments; no key search, collision
catalog, pricing finding, causal PGU attribution or global bound follows yet.

## Qualification and retained evidence

Frozen source `cfe6971929cea0df658c76a30e1aecacef95e6f2` changes seven files (+967/-3).
Author 21 focused unit tests, independent 21 pure falsification checks, four additional pure
boundary checks, five author and three independent pre-runner invalid-CLI cases passed.
Metadata/tests/host linking used actual rustc 1.95 and 447 cached library artifacts; this is
not a Cargo rebuild attestation. Nine estimator package files means five Rust files plus
Cargo.toml/Cargo.lock/README/.gitignore, not nine compiled Rust files. No SDK/helper/guest was compiled.
Preparation/compiler/artifact-name failures remain retained; they caused no actual diagnostic retry.

| Artifact identity | SHA256 |
|---|---|
| b76 ELF | `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034` |
| New diagnostic executable | `dfaee7ac41099df02e8a66d9b15b6c4b53d539d1a7e051a67b7180c64d6a0992` |
| `sp1-register-observer-source-review-20261104.json` | `3d73883e58db24d32563993b786f814c9d6c2a31a32e17fb3b45f06eede8115f` |
| `storage-register-watch-profile-20261104.md` (source-only preparation) | `ffdf3e699c48d2686a98835cc5fa70385c9472eaf885a9434d7020998969b4be` |
| `storage-register-watch-plan-20261104.json` (400M) | `ec752035f7a3fd7e47168109cc203682bde4a54d445c9c3fef5b5f495c7d73be` |
| Additional `plan.json` (1B) | `fc78bd3498e026a2684405f33125d6af994313c6257753047a834d8b7836123c` |
| Initial `final-artifact-manifest.json` (59 members) | `ed75f5be8f3371a17307f77cd2d323fdaa0693808917c3e8fa90ebf7a6b0e283` |
| Initial `final-readback.json` | `e288474c656a6919330b5addfce137f8a1b45675c46d6c505b7eb2460add8096` |
| Additional `final-artifact-manifest.json` (24 members) | `f63e0ff66d237f95c071943d20cd624da9b07a9ddf830e865c360e44dd075399` |
| Additional `final-readback.json` | `14993219d095d35f8568c71ce3ee5af29cbfe2e3caef772b612f6871896a6fb3` |
| `sp1-register-observer-root-closure-20261104.json` | `7b6c954c7c48d4fa4c059ad925c9aaf6effe8d2dd6fdb3c53db6ab035c39a7df` |
| Private `sp1-register-observer-evidence-20261104.tar.gz` | `a336fd5d64c6d56adb77919713ca6e998fbca85ac6a26907441a5ac4432bafc3` |

The private recovery archive is 738421 bytes, 214 regular members / 5282146 payload bytes;
all streamed member hashes passed without extraction. Four generated artifacts are excluded;
two runtime and 447 cached library artifacts remain external. It is not a standalone public Atlas fixture.
Initial/additional manifests independently close 59+24 members; old warm214/225, R, EXP,
original1998 measurer and b76 identities remained unchanged. Prior pricing cohorts stay separate.
Current SDK6.3/b76 conformance provides no SDK6.8 production parity, proof certification,
all-mode/operand bound, natural admission-usability claim or whole-PR readiness assertion.
