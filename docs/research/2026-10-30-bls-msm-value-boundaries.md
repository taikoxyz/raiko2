# BLS G1 MSM value-boundary experiment

Search for proving-cost underpricing using actual SP1 PGU / new node-known zkGas,
with the existing model 537a965e and experimental b76baf2e program. This is a
finite value/batching cohort; EVM-gas saturation remains deferred.

The exact program uses Arkworks 0.6. A single-point MSM uses binary scalar
multiplication; batches use signed-window MSM with special handling for small
positive/negative scalars. Thus a dense binary scalar is not necessarily costly
after batch recoding. Source inspection motivates sampling, not a deficit claim.

Use K=1/4/16 distinct input points `P_i=i*G` and four scalar patterns: one,
`(7<<252)-1`, alternating `0x55` bytes, and the valid scalar derived from
`SHA256("bls-g1-msm-scalar-1") & (2^255-1)`. The deterministic random value is
`0x70efa1463d7490ae8e43d134c3f832d1373383903e51edc8af34e73642855062`.
For shared scalar s, independently compute the output as
`[s*K*(K+1)/2 mod r]G`, and cross-check against pointwise multiplication/addition.
Also test K=16 with per-point hash-derived scalars, reduced modulo r.
Distinct inputs do not guarantee that every intermediate addition avoids a special case.

The 18 plans are twelve single calls (three K values times four shared patterns),
three zero-call shared-random controls, K=1 dense/random cases repeated four times,
and one K=16 mixed-random single call. Native precompile gas is source-derived
12,000/38,256/129,984 by K, including protocol batch discounts; total quote and
charged user gas are measured separately.

Add one fixed `BlsMsmValues.sol` runtime, `bls_msm_values.py`, focused tests and
usage text beside the existing producers. Calldata is count, K, K point/scalar
pairs, and expected output. Each pair is x64/y64/scalar32 bytes; returned output
is x64/y64. Copy input once, preallocate separate output, clear all four output
words per call, check status and exact 128-byte returndata, then compare all
four words. Validate count, K and payload length before calling 0x0c. Count bound
2048 exceeds the legal transaction call count at the minimum 12,000 gas/call.

Keep canonical ProgramRequest/GuestInput, guest, tariffs and resource guards unchanged.
Require curve/subgroup/encoding checks, independently checked expected points,
positive and negative native tests, quote operation-price comparisons, and independent
review/verification before SP1. Retain controls, failures, full quotes and literal
rankings. Fixed data belongs in Atlas; execution tools remain in Raiko.

## Completed observations

All 18 plans completed actual SP1 execution under the frozen program and model.
There are no failed or pending cases and no complete-quote deficits. For each K,
the four shared-scalar single-call inputs have identical observed opcode maps and
operation prices. Their actual work differs with the scalar and batching path:

| Points per call | Scalar pattern | Actual proposal PGU | Node-known PGU | Complete quote PGU |
| --- | --- | ---: | ---: | ---: |
| 1 | one | 161,794,223 | 105,436,830 | 260,378,916 |
| 1 | binary-dense | 164,722,738 | 105,436,830 | 260,378,916 |
| 4 | binary-dense | 166,308,313 | 311,588,546 | 466,530,633 |
| 4 | alternating | 169,996,990 | 311,588,546 | 466,530,633 |
| 16 | binary-dense | 175,088,104 | 1,031,863,279 | 1,186,805,366 |
| 16 | alternating | 184,856,370 | 1,031,863,279 | 1,186,805,366 |
| 16 | mixed-random | 184,713,072 | 1,031,863,279 | 1,186,805,366 |

All three repetition increments are covered. The largest actual/node increment is
0.036808878: K=1 binary-dense calls one to four add 10,953,032 actual PGU versus
approximately 297,564,955 node-known PGU. This finite cohort supports no tariff
increase. The existing overquotes remain explicit; this experiment does not tune them.

The literal actual/node maximum is the K=1 random zero-call control, approximately
25.826164. Its actual includes proposal startup, absent from the node subtotal.
Retain the literal ranking alongside complete quotes and matched increments.

Independent curve, subgroup and pointwise MSM checks, native/output checks, raw
SP1 joins and archive review passed. Peak process-tree PSS was 1,260,619,776 bytes;
all 72 exact owned resources were confirmed absent in the host namespace. These
are execute-only observations, not proof or GPU-throughput measurements, and do
not establish a bound over arbitrary curve points/scalars or saturated blocks.

The immutable data-only cohort is in [Atlas #14](https://github.com/taikoxyz/raiko2-atlas/pull/14).
Local integrity checks and the unchanged workflow against the published Release
verified all 316 archived files. Source checks use the immutable producer revision
rather than the subsequently evolving host constructor.
