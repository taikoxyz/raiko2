# ECMUL workload under EVM-only admission

This experiment asks two separate questions: whether the new node quote covers increasing
actual SP1 workload, and whether legal EVM gas alone keeps that workload acceptably small.
It retains the literal `actual SP1 PGU / new node-known zkGas` metric; paired increments and
complete quotes are additional diagnostics. These eleven cases are deliberately unsaturated
capacity probes, not a search certificate for the maximum.

## Execution and controls

The [producer](../../bin/zkgas/bench/cases/precompile_workload.py) emits one fixed Solidity
runtime and 128-byte calldata. Each loop calls BN254 ECMUL at address `0x07` with `G=(1,2)`.
It clears both output words, checks call success, exactly 64 returned bytes and both expected
coordinates. Independent affine scalar multiplication supplies the expected points. Exact
native STATICCALL and root RETURN counts reject skipped work and early exits. The two
zero-call controls use the same setup, memory allocation, runtime and selected program.

| Scalar | Reduced bit length | Set bits | Current scalar work selector |
| --- | ---: | ---: | ---: |
| Group order minus two | 254 | 127 | 381 |
| `(1 << 253) - 1` | 253 | 253 | 506 |
| `(3 << 252) - 1` | 254 | 253 | 507 |

The pinned Alethia [bn254_work_units](https://github.com/taikoxyz/alethia-reth/blob/0f30f488c88635e38932843e67af2d918319f7e7/crates/evm/src/zk_gas/runtime.rs#L436) reduces the scalar modulo the group order, then counts
bit length plus set bits. A reduced scalar has at most 254 bits. The only 254-bit scalar with
254 set bits exceeds the group order, so the maximum selector is 507; the chosen dense254
scalar attains it. This bounds that selector only, not actual proving cost over every possible
point, field value, composition or program. Native opcode maps at equal loop counts match
across scalar families, while the model correctly distinguishes their scalar selectors.

The fixture constructor retains its 15M user-transaction and 31M block gas settings; these
are tool choices, not protocol limits. Its count guard `4096` exceeds any successful ECMUL count
within Osaka's 16,777,216 gas cap even before wrapper and intrinsic costs. Independent native
qualification checked all scalar outputs, `N=0/1/4` repetition maps, corrupted coordinates,
calldata lengths, count `4097` and a direct `N=4096` EVM-gas-exhaustion case. The exhausted case
reverted honestly; it is not assigned an actual SP1 observation in this pilot.

All eleven SP1 plans use `evm-gas-only`, unchanged model
`537a965e51dd0416e3472a31cca8b067a85723fe4bbd08c52027ef3ca73f0e3b`, experimental ELF
`b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`, and the same declared
SDK 6.3.0 program manifest. Guest source revision is `4580faf7`; case producer revision is
`66ef0710`. The latter adds no guest source or tariff changes, so this experiment reuses the
already built program. Canonical native, quote and SP1 input/public-output identities join.

## Completed measurements

All eleven plans completed actual SP1 execution. There are no failed, pending or complete-quote
deficit observations. Each row is one complete one-block proposal; user gas excludes Anchor,
while actual PGU and complete quote include the whole proposal. Every Anchor uses 167,355 gas.

| Scalar | Calls | Charged user EVM gas | Actual SP1 PGU | Node-known PGU | Complete quote PGU |
| --- | ---: | ---: | ---: | ---: | ---: |
| r-minus-two | 0 | 25,160 | 161,024,115 | 6,175,381 | 161,117,212 |
| dense254 | 0 | 25,160 | 161,024,159 | 6,175,381 | 161,117,212 |
| r-minus-two | 1 | 29,377 | 343,111,347 | 229,482,783 | 384,425,126 |
| dense254 | 1 | 29,377 | 403,031,832 | 301,672,719 | 456,615,062 |
| dense253 | 1 | 29,377 | 402,545,688 | 301,099,783 | 456,042,126 |
| r-minus-two | 4 | 48,499 | 891,840,887 | 899,368,505 | 1,054,310,847 |
| dense254 | 4 | 48,499 | 1,132,873,063 | 1,188,128,249 | 1,343,070,591 |
| r-minus-two | 16 | 124,987 | 3,083,505,057 | 3,578,911,390 | 3,733,853,733 |
| dense254 | 16 | 124,987 | 4,041,998,502 | 4,733,950,366 | 4,888,892,709 |
| r-minus-two | 32 | 226,971 | 6,005,985,953 | 7,151,635,237 | 7,306,577,580 |
| dense254 | 32 | 226,971 | 7,922,688,775 | 9,461,713,189 | 9,616,655,532 |

All eight adjacent same-scalar repetition increments are covered by node-price increments;
the largest actual/node increment is 0.823325261. The literal actual/node maximum is
26.075178034 in the dense254 zero-call control, where proposal startup is absent from the
node denominator. It remains a separately labelled result, not an operation-level ratio.
Node/full prices and raw actuals are retained rather than assigning proposal actuals to blocks.
Peak process-tree PSS was 4,546,978,816 bytes under the unchanged 6GiB guard.

## Interpretation

A capacity judgment must state its assumed prover bandwidth. Under the previously discussed
illustrative budget of 10M PGU/second for 360 seconds, the proposal budget would be 3.6B PGU.
The dense254 `N=16` case already measures 4,041,998,502 PGU with only 124,987 charged user gas.
Its whole derived block, including Anchor, consumes 292,342 gas. The new node quote is
4,733,950,366 PGU and complete quote 4,888,892,709 PGU: this is expensive work that the model
recognizes, not an observed pricing deficit. The bandwidth assumption is not a measured
hardware result, and local execute-only estimation does not certify proving latency.

Even this unsaturated observation is enough to reject the specific hypothesis that the
current legal EVM envelope alone guarantees workload below that illustrative budget for
this program. It establishes a lower bound on achievable workload, not a maximum, and must
not be extrapolated linearly to a full gas limit or a multi-block proposal as measured data.
The completed dense254 `N=32` observation strengthens that lower bound: 7,922,688,775 PGU
at 226,971 user gas, or 394,326 whole-block gas including Anchor. It also exceeds the 7.2B
illustrative budget for 20M PGU/second over 360 seconds. Its node quote remains conservative
at 9,461,713,189 PGU. These figures do not establish a saturated-gas maximum.
Retain zkGas admission as a separate question from EVM validity. No production limit changes
or pricing coefficient changes follow from this pilot.

Broader point/value axes, other precompiles, opcode/state/payload combinations, refund-aware
gas saturation and complete multi-block proposal measurements remain follow-ups. See the
[execution-profile checkpoint](2026-10-29-execution-profile-admission.md) for the independent
protocol-envelope inspection and prior RETURN/copy cohort.

## Retained evidence and validation

The data-only [Atlas #12](https://github.com/taikoxyz/raiko2-atlas/pull/12) retains all eleven
canonical inputs, raw reports, manifests, controls and summaries. Its immutable
`fixture-unzen-ecmul-capacity-pilot-v1` Release was downloaded again and passed the unchanged
Atlas integrity CI body for all 197 files. Reference ELF and host binaries are excluded;
consumers rebuilding the program must record its identity and qualify their own controls.

The six focused compiler/native tests and eighteen independent native cases passed. Final
`workload.py verify` passed all eleven attempts. Independent adversarial review and behavioral
verification confirmed raw PGU normalization, full input/output/source/profile joins, rational
rankings, gas separation and all eight increments. All 44 exact owned resources were verified
absent in the host namespace. A separate independent archive pass checked 132 unchanged raw
files, 45 explicit original-SHA-bound projections, 22 compressed canonical inputs and complete
checksum closure. Remote Actions status remains separate from the local and published-asset
checks; no production guest rebuild, deployment or automatic merge was performed.
