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
