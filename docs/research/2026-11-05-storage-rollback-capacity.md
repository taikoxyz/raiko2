# Storage capacity through ordinary rollback

All eight selected rollback/reaccess workloads are covered by both literal node-known and
completed-proposal pricing under unchanged model5f. Actual SP1 workload is 212.06M–268.40M
PGU; node quotes are 1.912–5.078 times actual. No pricing change follows. Auxiliary phase
profiles remain incomplete in all eight inputs; this does not invalidate their complete
canonical GasVM measurements or turn the diagnostic into a successful phase certificate.

## Controlled workload

Each synthetic post-Unzen proposal contains one block, its Anchor and two signed user CALLs.
Each user follows two DELEGATECALL wrappers to a shared leaf, preserving a distinct top
account's storage context. Only five reachable contracts are configured. The complete world
has nine initial accounts and ten final accounts, including the ordinary Anchor/sender changes.

Each storage owner starts with phase slot0=65537 and K consecutive body slots starting at
65537, all equal to1. K447/448/895/896 therefore means 448/449/896/897 distinct owner keys.
The leaf checks, writes1→5, cleans5→1 and reverts; the parent catches failure and checks every
original value. Optional caller prewarm occurs before the lower checkpoint. Both top user
receipts succeed and effective/applied refunds are zero. R0 omits additional dirty-loop pairs;
it still performs 6K+2 leaf SLOADs and 2K SSTOREs per owner.

These sequential raw keys have distinct low32 values, including phase0. No guest hash seed,
bucket or tag search is performed. Ordinary rollback restores warmth/values while retaining
loaded slot entries; it is not physical clearing/refilling. Without prewarm the postcheck is
cold; caller-prewarmed slots remain warm across the lower rollback.

The previous four-user fixture would exceed the 30M research budget for large cold cases.
The explicit two-user schedule exercises the same per-user path, with independently recomputed
full-R signed envelopes and all ordered admission prefixes. Maximum measured block gas is
15,113,727. EIP150, the 12M child request, 15M per-user fixture cap, EIP7825, 31M header and
30M charged-block checks remain. This panel is not a full30M packing experiment. Full-R
factory endpoints were calculated and four tested at the byte level, but no full-R input
was measured with SP1 in this panel.

## Three workload columns

The fixed identities are model
`5f93909ea1372eb6e10fa2b599c6087aafd4ec493ba28eb1cea4c4abb3708a11`, ELF
`b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`, SDK6.3,
native constructor `ad0d78dd…`, quote `1390ee77…` and observer `dfaee7ac…`, as in the
[pure-growth panel](2026-11-05-storage-capacity-boundaries.md#inputs-and-measurement).
Actual PGU is complete `floor(raw ExecutionReport.gas * 10 / 191)` under `evm-gas-only`.
The node denominator is `quote.node.sum_of_block_known_pgu`; proposal pricing is
`quote.prove.total_pgu`. Raw Unzen difficulty remains a different unit.

| Body K | Caller prewarm | Block EVM gas | Node PGU | Proposal PGU | Actual SP1 PGU | Actual minus node |
| ---: | --- | ---: | ---: | ---: | ---: | ---: |
| 447 | No | 7,654,939 | 405,372,356 | 577,979,962 | 212,060,980 | -193,311,376 |
| 447 | Yes | 6,032,283 | 413,711,374 | 586,318,981 | 214,634,499 | -199,076,875 |
| 448 | No | 7,671,551 | 406,238,312 | 578,873,966 | 212,447,746 | -193,790,566 |
| 448 | Yes | 6,045,273 | 414,595,915 | 587,231,569 | 215,049,048 | -199,546,867 |
| 895 | No | 15,097,115 | 1,332,468,915 | 1,522,508,986 | 262,414,592 | -1,070,054,323 |
| 895 | Yes | 11,851,803 | 1,349,133,973 | 1,539,174,043 | 267,715,422 | -1,081,418,551 |
| 896 | No | 15,113,727 | 1,333,936,983 | 1,524,005,102 | 263,143,786 | -1,070,793,197 |
| 896 | Yes | 11,864,793 | 1,350,620,625 | 1,540,688,744 | 268,397,198 | -1,082,223,427 |

Both estimates cover all eight actuals; no positive node/proposal gap or >2B-within30M deficit
appears. The minimum node margin is 193,311,376 PGU. Proposal quotes are 2.725–5.802 times
actual. This conservatism on selected synthetic workloads does not measure historical
transaction admission usability or justify rescaling the whole model from these cases alone.

Caller prewarm reduces EVM gas but adds 2.574/2.601/5.301/5.253M actual PGU for the four K
values. Node pricing adds 8.339/8.358/16.665/16.684M PGU respectively. Thus lower EVM gas
is not treated as lower proving work here. These are whole-input contrasts with their own
seeds and payloads, not an isolated opcode cost allocation. Adjacent K inputs show no workload spike; their
different seeds/layouts mean the differences do not isolate resize cost.

The existing journal formula accounts for this path. With W=0/1 for prewarm, per owner:

```text
N = K + 1; M = 1; C = ceil((N + 7*M)/8)
A = 9*K + 3 + W*(K + 1)
J = 4*K + 2 - W*(K + 1)
block journal work = 2*C*(A + N + J) + 54
                   = 2*C*(14*K + 6) + 54
```

The observed quote work units are 714,150 /715,746 /2,833,190 /2,836,354, identical across
prewarm variants. They are priced once at the existing384 PGU/unit tariff; the proposal-once
journal term is zero. Cold occupied reaccess still contributes lookup/rollback work, and
the lower revert does not erase attempted computation. C is a conservative comparison-work
proxy, not a measured hash-probe count or a global PGU bound. No journal implementation is replaced.

## Diagnostic failure retained separately

All eight instruction observations complete, but register samples are capped. The original
strict phase reader returns incomplete for every input. Its two standalone drain markers
have zero hits; static instruction/call-target checks had not proved that this specialization
was the executed caller. The available raw registers cannot identify the actual outer caller.
Do not describe a guessed alternative caller as confirmed.

The four cold samples retain the exact first-owner late key sequence, earlier matching seed,
same map/control, matching pre/post masks511/1023/1023/2047 and 48 sampled occupied returns
(one before revert, 47 after). These partial facts do not upgrade the failed full phase gate.
Warm samples fail the earlier same-map seed condition: early and late struct addresses differ.
An early seed value is compatible with the later hash arithmetic, but early control identity
was not captured. Account movement is a hypothesis, not an observed cause. Warm profile mask
fields remain null; raw header loads are separately available without seed qualification.

Complete restoration counters independently match: Changed=4K, Warmed=2(K+1) without caller
prewarm and0 with it. Independent byte/frame execution, authenticated roots and these counters
remain separate from the incomplete register-based phase profile. No actual was rerun to
repair diagnostics, and none of the original plans/readers/reports was retrospectively changed.

Deferred diagnostic TODO: sample the actual Changed restoration branch and the common return's
AFTER-load RA, then correlate clocks/frame SP to identify the real caller. Move the seed window
to the same late hits and check `query < seed < header` in that call. These changes can retain
2048 register samples and400M instructions; they need their own source/negative-path review
before a future experiment. They are not a prerequisite for the current complete PGU results.

## Retained tooling, checks and scope

The [reusable family](../../bin/zkgas/bench/README.md#explicit-rollback-capacity-keys-and-user-counts)
supports strict explicit K1..1024 and warm-family `user_count=4` bounded1..4. Original warm32
and dirty16 complete rows and request bytes are unchanged. New schedules provision only
reachable chains, recompute all inherited counts and envelopes, and authenticate concrete
gas/refunds/routes. The wrapper interpreter has an explicit65,536-step bound.

- Focused Python suite:21 tests,19 passed and2 optional old-native gates skipped. Root and
  independent reviewer separately ran the six new key checks successfully.
- Independent original-source comparison verified all48 default rows. The old independent
  frame/leaf interpreter executed all16 selected user paths, separate from producer gas helpers.
- All eight native inputs,16 signatures, three receipts each, owner slot values, code hashes,
  full9→10 account roots and5f quote arithmetic passed independent readback.
- Thirteen phase-reader tests and20 native/actual-adapter negative tests passed. Static marker
  reachability nevertheless proved insufficient; the actual diagnostic failure stays recorded.
- All eight complete actual reports, input/build/options identities and guard receipts passed
  independent readback. Root separately confirmed all32 unique owned SHM names absent on the
  original host. Maximum PSS was1.731GiB; longest run16.442s, within the unchanged6GiB/600s,
  192MiB output,16MiB log and576MiB free-disk limits.

An independent prelaunch check caught a matrix that inherited the older catalog's model ID.
The rejected metadata is retained; the matrix was corrected to5f before any stage started.
Requests were unchanged, and no native/SP1 stage was retried. Existing catalog defaults are
historical identities; an experiment must bind its model explicitly.

Frozen matrix SHA is `15b1b65ed331af12a694c823f3a1214d0ff21af956937deaac5b77c44436fc54`;
measurement summary SHA is `effaf20322afd95cef3edd9e191d9c71330a156a78823aa8fdffa388f4daa866`.
These opened historical-program cases do not certify current production SDK accuracy.
Fixed data belongs in Atlas. No guest/SDK/broad Cargo build, proof generation, network
transaction, merge or deployment occurred. The account-journal correction and future L1
occurrence sampling remain deferred. Other compositions and a global workload bound remain open.
