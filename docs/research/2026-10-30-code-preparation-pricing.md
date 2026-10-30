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


## Actual successor replay result

The frozen policy above was not changed after the prospective run. The implemented
successor is model
`a2988920125e304121acb629b9b96fd1ad85ee707c89978240408c81e2a42f3e`,
Reth correction `8da28fd02e44d81d2c84725fbd082f73cd48428e`, consumed by Raiko
`e3ab03454efeb2b0f7462a237e03ddcaffc76fac`. All108 saved canonical inputs were
actually natively repriced; SP1 was not rerun for repricing.

| Cohort | Measurements | Prior full-quote deficits | Successor full-quote deficits |
| --- | ---: | ---: | ---: |
| Initial creation | 44 | 1 | 0 |
| Development followups | 38 | 11 | 0 |
| Existing-code loading | 10 | 6 | 0 |
| Prospective validation | 16 | 9 | 0 |
| Total | 108 | 27 | 0 |

There are102 distinct canonical inputs, six known reference replays,128 block
occurrences and3342 transaction occurrences. The16 prospective measurements
contain14 fresh inputs excluded from fitting and two known reference replays.
The27 prior deficit rows represent26 distinct inputs; neither duplicates nor
controls are filtered away. The smallest successor full-quote margin is478,893
PGU, on the one-empty-create control.

Examples in PGU (whole one-block proposal actuals; node excludes shared startup):

| Case | Actual SP1 | Prior node | New node | Prior full | New full |
| --- | ---: | ---: | ---: | ---: | ---: |
| Load24K JUMPDEST-tail code, user gas21000 | 162,336,784 | 6,161,863 | 8,098,309 | 161,103,694 | 163,040,140 |
| Deploy the same24K code four times | 163,788,369 | 7,116,559 | 12,231,873 | 162,494,230 | 167,609,544 |

The ranking metric remains literal **actual SP1 PGU / new node-known zkGas**.
The largest ratio in this finite set is24.698411656 on the one-empty-create
control: actual160,968,292 divided by node6,517,354. A shared proposal-startup
charge is absent from that denominator. Full-quote coverage and matched increments
are separate diagnostics; zero full-quote deficits does not prove a universal
node-admission bound or replace that ranking metric.

Positive actual differences at identical selectors/prices remain explicit:
JUMPDEST initcode tails at4096/49152 bytes add16,438/205,860 PGU; MCOPY-filled
JUMPDEST runtime at4096/24576 adds21,714/102,474 versus equal-length zero controls.
The new conservative byte terms cover these measured full proposals, but do not
make every positive content increment itself priced. Future composition tests
should amplify repeated initcode preparation, including empty deployed output,
internal CREATE/CREATE2, ancestor rollback and mixed code/state work. The existing
B3 carried-code caveat still applies; it was not used to fit deployment units.

## Actual historical replay

The same new quote executable repriced50 retained Mainnet proposals /9600 blocks.
These retain SDK6.1 /ELF416f actuals and are deliberately separate from the108
SDK6.3 /ELFb76 observations.

- Actual sum:69,980,978,094 PGU.
- Prior full-quote sum:145,920,113,551 PGU (**2.085139670× actual**).
- New full-quote sum:150,234,443,229 PGU (**2.146789704× actual**).
- Full-quote deficits:0/50 both before and after.
- Node-known sum:78,776,601,035 →83,090,930,713 PGU.
- Unchanged exact/node unique-code bytes:69,583,225 in total.
- New attempted preparations:two events /4792 bytes; their contribution is169,728
  PGU. Resource uplift is exactly69,583,225×62 =4,314,159,950 PGU.

Every old block/proposal component except the declared resource uplift and new
preparation term remains unchanged, including public outputs, selectors, Unzen
values, operation curves, MPT, payload and startup. The result confirms the earlier
resource-only projection with the observed preparation term added; it preserves
rather than overturns the roughly2× historical conclusion. It is native replay
against retained actuals, not fresh proving or hardware-throughput certification.

## Validation and retained identities

The frozen new quote SHA256 is
`0ead8567bab74b7277ed729010c9eb8b149dc50f822bcd039e105983f0100380`.
Its complete490-file source inventory is
`cdf7306c6311dc7133896b676b3d8ce43509a85a3140ec8be5d881b57490129f`.
It was built at a modified e42eda5d tree with null embedded revision; a later
association verifies every inventory entry against committed e3ab0345 blobs.
This records source association without rewriting the original build identity.
The frozen native constructor still has its484-file inventory, old quote481,
and experimental program67; those distinct identities are not merged.

Independent Reth source review and behavioral verification passed:167 author EVM
tests,26 Python checks and separate public-API probes covering95 real EVM
transactions, real block receipts/state/bundle, collector migration/discard and
selector failure atomicity. Plain and inspected execution agree; old Unzen
behavior remains unchanged. Narrow no-default and strict targeted Clippy checks
passed; no full default-net Reth validation is claimed.

Raiko checks passed:

```text
cargo test --locked --offline -p zkgas --test compare code_preparation_
cargo test --locked --offline -p zkgas -p raiko2-zkgas-trace --lib --tests
cargo test --locked --offline -p zkgas --features fixture --test fixture --lib
cargo test --locked --offline -p zkgas -p raiko2-zkgas-trace --features zkgas/fixture --lib
cargo clippy --workspace --locked --offline -- -D warnings
cargo fmt --all -- --check
```

These cover6 focused comparator tests,107 zkGas tests,25 trace tests and37 fixture
integration tests. Missing/forged selectors and costs, negative/overflow values,
attempted-prefix coverage, foreign-model domain validation and proposal double
charges are rejected. Three guest lockfiles also passed locked/offline metadata
parsing. Test source was independently reviewed. No guest was rebuilt, no admission
mode was enabled and no merge/deployment followed; production artifact/source
alignment remains a separate Draft PR gate.


The final independent consumer audit is content-bound by SHA256
`b412b63049e283d283879c780edcade7c51235af70bc4c051c9b0bd3b4b412bf`.
It verifies all108 controlled quote rows against independent native semantics,
the50 historical input payloads and quote deltas, every490-file source/archive/Git
blob association, literal ranking and all83 matched diagnostics. Historical create
counts are the native observations reconciled against domain/ownership and full
price closure; this does not claim a second independent per-frame trace of every
historical transaction. Original actuals and allfour raw campaign audits remain
unchanged. The data-only record is preserved under Atlas fixture
`unzen-code-preparation-v1`; consuming regression chooses execution strategy.
