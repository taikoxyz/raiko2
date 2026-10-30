# Code preparation pricing candidate

## Frozen development evidence

The unchanged3794 model was tested against92 execute-only SP1 observations /88
canonical inputs:44 initial creation cases,38 development followups (35new plus
three known replays), and10 existing-code-loading cases (nine new plus one known
replay). The same b76baf2e ELF /SDK6.3 and canonical native/quote/SP1 joins apply.
All three raw campaigns passed independent integrity review and all368 exact
owned shared-memory objects were absent after execution. Original results stay
immutable, including18 underquoted measurement rows (17 distinct inputs).

- Initial44 summary: `0edfb0a81ec28210e6d69a4e1733550e13ee2735f63500b3d19dd8d6f8baf2c5`;
  raw audit: `bdb2811484b89d01a309305e4fc3838c1a6fbb0390ab27a37f703e9fd631cf09`.
- Followup38 raw audit: `99950de03511a2f1566eedca9392010d258c082295335518f2a287f1ae763554`.
- Code-loading10 raw audit: `bd907d6a783c4d7c9b15461b419d00d0c32528cd2405935be1f0f117932d7747`.

Existing runtime24K, leading STOP, empty calldata and21000 charged user gas:
JUMPDEST-tail actual162,336,784 PGU versus161,103,694 full quote. Same runtime
successfully deployed four times: actual163,788,369 versus162,494,230 full quote.
Ordinary CALL returning24K is covered. Bytecode preparation is broader than RETURN.
Do not infer absolute pure hashing time from these differences.

## Candidate pricing policy, fixed before prospective validation

Keep all operation curves, memory, payload, account/storage/hash, MPT, startup,
authorization/access-list tariffs and the global discount unchanged, except:

1. Change the normalized effective `unique_code_bytes` unit from2 to **64 PGU/B**.
   This prices required code materialization/preparation represented by existing
   per-block unique code resources. It also applies to observed initcode resources;
   it is not an arbitrary charge on calldata or all witness bytes.
2. Add **8192 PGU per nonempty accepted create-frame code preparation**, plus
   **32 PGU per prepared byte**, counted per actual occurrence, not per hash.
   Empty output contributes zero. Failed size/prefix/deposit checks contribute zero
   because they return before Bytecode preparation. Work later reverted by an
   ancestor still counts. Cover top-level CREATE and internal CREATE/CREATE2.

The resource-byte policy uses all positive-byte/positive-work pairs in the10
loading observations, subtracting unchanged nonbyte pricing. The largest required
final unit is exactly242969/4096, or59.318603515625 PGU/B. Ceiling60 and upward
power-of-two rounding select64. This is finite empirical conservatism, not a25%
headroom rule or universal value-independent bound.

The nonempty-event component uses the one-byte/empty-runtime matched pair: actual
increment5375 PGU, existing exact-price increment1193.300680307025542727 PGU.
A25% workload-increment margin leaves5525.449319692974457273 PGU before any new
byte fee. Ceiling5526 and upward power-of-two rounding select8192.

For repeated identical24K code, use within-block difference-in-differences against
same-count empty-code controls, subtracting the N1 difference. This removes shared
transaction/account work and the one-time unique-byte term. With8192/event, the
25%-margin residual byte constraints are25.00906189093123 (N2) and
25.138897231643035 (N4). Ceiling26 and upward power-of-two rounding select32 PGU/B.
Do not impose25% on unrelated existing per-transaction work or fit from nonminimal
multiblock carried-code data. Small equal-selector content differences remain
visible; every point must also pass actual successor native repricing.

A resource-only arithmetic projection on the retained50 Mainnet proposals /9600
blocks changes full/actual2.085139670 to2.146787278 before the new deployment term.
These historical actuals retain SDK6.1 /ELF416f identity, separately from the92
SDK6.3 /b76 development observations. This is **not an actual successor replay**.
Reprice those inputs and check the new counters before claiming historical
compatibility or unchanged conclusions.

## Implementation contract

Use one checked accumulator helper and the common real
`TaikoEvm::frame_return_result` point, before delegating to the inner EVM. Both
plain and inspected execution reach it after accepted creation has prepared code.
Read successful Create outcomes with an address, count original nonempty output
length, and sum event/byte selectors transactionally in committed/attempted scopes.
Do not derive occurrence work from final state, code hashes or EVM deposit gas.

Access whichever single collector is active. Preserve direct generic
`TaikoEvm<NoOpInspector>` use without adding a new Inspector trait bound: reuse the
existing execution-hook pattern or an optional typed inspector-collector accessor.
No duplicate collector or frame-start marker stack is needed. Disabled/Unzen
execution, state roots, receipts and legacy meters must remain unchanged.

The common hook runs after inspector completion callbacks. Quantitative pricing
therefore requires a semantics-preserving inspector, as used by current quote and
fixture observers. An inspector that changes successful output/status afterward
is outside this quantitative contract; do not claim accurate counts for arbitrary
mutating inspectors. Supporting those would need an earlier authoritative hook.

The new normalized contribution is additive once: committed events/bytes under
node, attempted events/bytes under prove block pricing, zero in proposal-once.
Validate both selector consistency and exact count-to-price totals; missing new
current-schema fields must not silently become zero. Regenerate model artifacts
through a deterministic successor generator and preserve the3794 parent/evidence.

## Prospective validation, excluded from fitting

Freeze before execution a16-row set:14 new candidates and two explicit known
references. New loading lengths8192/16384 each use zero/JUMPDEST/PUSH32 tails;
creation probes use output2048,12288, same-code24576 repeated three times,
MCOPY-generated JUMPDEST8192, and one-byte output repeated16/128 times. Add a CALL
factory that internally creates24576-byte code, with outer STOP and outer REVERT
variants. References are the old one-call STOP and one-create empty-runtime cases.

Validate every canonical input natively first, then execute the same frozen program.
Check required counter ownership explicitly, especially nested work surviving outer
REVERT. Do not refit from these prospective results. Reprice all92 original rows
and50 historical proposals using the actual new quote executable. Keep any failure,
underquote, content sensitivity and overquote visible. No guest rebuild, real proof,
hardware-throughput certification, production activation or automatic merge follows.
