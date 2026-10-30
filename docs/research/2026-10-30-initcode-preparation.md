# Repeated initcode preparation experiment

## Goal and frozen baseline

Search valid post-Unzen proposals for large literal actual SP1 PGU / node-known zkGas.
Focus on repeated long initcode producing empty code. Full proposal quotes and matched
increments remain separate diagnostics; preserve controls, rejections and failures.
Freeze model a2988920125e304121acb629b9b96fd1ad85ee707c89978240408c81e2a42f3e.

REVM43 processes fresh initcode on each successful CREATE/CREATE2 frame, even when the
first instruction is STOP and deployed output is empty. The64/B unique-code resource
term deduplicates hashes per block;8192/event+32/B applies only to accepted nonempty
output. However confirmed CREATE/CREATE2 wrappers already cost about3.12M/3.75M PGU per
event. Missing a length-times-occurrence selector does not establish underpricing.
The first stage measures whether the existing wrapper already covers the bounded work.

CREATE2 computes initcode keccak before destination collision checks; legacy bytecode
analysis occurs only after depth/balance/nonce/collision checks pass. A collision is an
exceptional halt that burns forwarded gas, so it is not a successful-analysis repetition.
Ordinary CREATE in the frozen plain guest does not add the Shadow collector's initcode
hash computation. Quote instrumentation must not be counted as frozen actual workload.

## Approach and alternatives

Add a Python case producer; reuse all existing Rust/native/SP1 tools. A small factory
constructs a repeated-byte initcode image in memory using MCOPY, then loops over CREATE
or CREATE2 with distinct salts. This avoids repeating a49K signed transaction payload,
embedding an oversized runtime, or growing the factory body with repetition count.
Top-level repeated deployment and calldata-backed factories remain valid other cases,
but introduce repeated or larger payload terms; they are not the chosen isolation here.

Children execute STOP, or a controlled prefix returning one zero byte, followed by an
unreachable zero/JUMPDEST/PUSH32-byte tail. Require every intended child address to be
nonzero. An outer REVERT happens only after every child completed; its spent work remains
visible even though accounts and factory nonce roll back.

## Frozen first-stage matrix

All24 cases: one block, one CALL user, empty calldata,15M user gas envelope, evm-gas-only
profile, and ordinary EVM/proposal validation. This is a finite boundary experiment,
not a gas-saturated maximum-capacity measurement.

| Group | Configuration | Rows |
| --- | --- | ---: |
| Core | CREATE/CREATE2 × initcode4096/49152 × repeats1/16/128; JUMPDEST tail; outer STOP | 12 |
| Content controls | Both families × zero/PUSH32 tail × length49152 × repeats16 | 4 |
| Zero repetitions | Both families × length49152 JUMPDEST image × repeats0 | 2 |
| Ancestor rollback | Both families × length49152 JUMPDEST × repeats16; outer REVERT | 2 |
| One-byte output | Both families × length49152 JUMPDEST × repeats16; child returns one zero byte | 2 |
| Empty initcode | Both families × length0 × repeats16; outer STOP | 2 |
| Total | Deterministic catalog and explicit reference links | 24 |

Zero-repeat controls still prepare the same parent memory image but execute no CREATE.
Expected route counts cover actual child CREATE/CREATE2, STOP/RETURN and outer REVERT.
Check the factory nonce: N on outer success,0 on outer REVERT. Caller transaction nonce
still advances for included REVERT. Nonempty output selectors must be N events/N bytes;
all empty-output cases must be0/0. Empty-code success is distinct from a failed creation.

Use same-family/shape reference links. The N0 control is only a49152-byte JUMPDEST
image; do not use it as a matched setup baseline for4096-byte or zero/PUSH32 cases.
Use each length's own N1 for repetition slopes and long/short differences-in-differences.
Report content changes, rollback and nonempty output as different dimensions, not
isolated pure-initcode pairs. CREATE2 addresses
change with initcode hash; its differences can include trie work. Keep every component
and actual user/block EVM gas. Repetition slopes and long/short differences-in-differences
are diagnostics, never assigned per-transaction actuals.

## Execution and retention

Qualify all24 natively before actual SP1 sampling. Freeze the producer revision/catalog
before measurements. Reuse native b0497f45, quote0ead8567, sampler1998fb85 and ELFb76baf2e
with SDK6.3 and their original source manifests. Producer source identity is separate
from native484/quote490/program67 compiled-source inventories. Python-only extensions
require no host Rust or guest rebuild.

Run sequentially using existing process-tree PSS/time/disk/log guards and exact resource
receipts. Verify the identical canonical input hash and public outputs across native,
quote and SP1. Preserve every failed/unattempted case. Independently audit results.

Do not adjust tariffs here. If this stage exposes underpricing, retain that evidence
and design the correction with fresh holdouts separately. If coverage is conservative,
record that negative result and move to the next composition surface. Larger repetitions,
two-user salt namespaces/cross-transaction rollback and explicit CREATE2 collision are
possible followups, not mixed silently into this matrix. EVM-gas saturation stays a later
TODO. No guest build, activation, merge or deployment is part of this experiment.


## Source evidence before measurement

The frozen program source/manifest binds its own Cargo.lock, independently of later host
pricing pins. Reviewed package sources are revm-handler43.0.3 (`frame.rs:259–334` for
prechecks and fresh initcode construction; `frame.rs:640–734` for output processing),
revm-interpreter43.0.2 (`instructions/contract.rs:26` for memory copy and3860/CREATE2 gas),
and revm-bytecode43.0.0 (`bytecode/mod.rs:132`, `legacy/analysis.rs:10` for per-occurrence
analysis). The registry sources were checked against their .crate archives and lockfile
checksums by independent source inspection. CREATE2 collision occurs before analysis but
after its initcode hash, and exceptional failure consumes forwarded gas.

The reviewed Reth a298 table's confirmed CREATE and CREATE2 wrappers use the maximum of
absolute, incremental and structural candidates. The roughly44K/46K incremental candidate
alone is not the final per-event price. Quote resource observation may hash initcode even
when the plain frozen CREATE guest does not; only measured guest PGU is the actual metric.


## Native qualification checkpoint

The24-case producer passed independent source review and deterministic decoding.
The author observed the missing-module RED before implementation; focused native-enabled
checks passed all24 native and current-quote cases. The full Python suite ran104 tests:
85 passed and19 optional/native tests were skipped. No Rust or guest build was performed.

Independent behavioral verification separately executed all24 requests and current-model
quotes, verified exact memory images, salts, completed routes, inclusion, receipt gas,
factory nonce and public-output/input identity. Three separate two-block state probes
verified child persistence and ancestor rollback; three negative mutations (loop off by
one, repeated CREATE2 salt and insufficient gas) were rejected without published outputs.
These probes are qualification evidence, not additional SP1 measurements in the24-row set.

Independent qualification audit SHA256:
`ba4d3661e6920b0906f2a53d431664c8f03491acec974c8c73fad93922fca3bf`.
Qualified original24-input index SHA256:
`1a3d82d5b9c5e70e26dbf3e5614f244e8144f022efd161cc5d44ba1792827bf6`.
Reuse those exact canonical inputs/native reports through the existing `measure` CLI,
then re-quote with the campaign's own corpus identity. This avoids rebuilding already
qualified blocks. At this qualification checkpoint, actual SP1 sampling and pricing conclusions were
still pending; the completed results follow below.


## Actual SP1 result

All24 canonical inputs were executed once by the unchanged SDK6.3 /ELFb76 sampler.
Producer a0a2a5dc502cbf3edcb693c7162125f24b4500b4 was committed before sampling;
source inventory492 remains separate from the frozen native484/quote490/program67
identities. Native input/report reuse and fresh campaign-key quotes passed all joins.
No pricing coefficient, Rust source, guest program or production setting changed.

There are24 distinct measured inputs,24 block occurrences and48 Anchor-plus-user
transaction occurrences. All completed, with no failures or unattempted plans. Full
proposal quote deficits are0/24. Full/actual ranges from1.003392899 to3.694925992.
The largest literal actual/node ratio is24.190301779 on the zero-create startup
control (161,053,151 actual /6,657,757 node); it is retained as a control, not hidden
or replaced with the complete quote.

| Internal work,128 successful children | Charged user EVM gas | Actual SP1 PGU | Node-known PGU | Complete quote PGU |
| --- | ---: | ---: | ---: | ---: |
| CREATE,4096-byte initcode | 4,159,731 | 172,209,962 | 408,197,079 | 585,276,550 |
| CREATE,49152-byte initcode | 4,533,251 | 242,891,500 | 411,186,631 | 588,266,101 |
| CREATE2,4096-byte initcode | 4,258,419 | 179,883,155 | 489,200,802 | 664,654,945 |
| CREATE2,49152-byte initcode | 5,713,283 | 330,757,339 | 492,190,353 | 672,132,104 |

These are whole one-block proposal actuals, including shared startup, not per-child
measurements. From N1 to N128, long CREATE actual grows81,189,337 PGU against
approximately398,244,392 additional node PGU; CREATE2 grows168,360,585 against
approximately478,615,210. All eight same-length N1-based repetition increments
and six same-parent-image N0 comparisons are covered by both node and full increments.
The existing per-event wrappers provide the measured budget headroom, so this round
requires no added initcode tariff or change to the global discount. The preceding
top-level deployment fixtures remain a separate regression surface; confirmed-spawn
pricing here is exercised inside a CALL factory.

Length/content sensitivity is real and remains explicit. Long/short repetition
DIDs cancel each length's N1 setup and show the following extra actuals while node
DID is exactly zero:

| Family | Repetitions | Actual length DID, PGU | Node length DID, PGU |
| --- | ---: | ---: | ---: |
| CREATE | 16 | 8,281,204 | 0 |
| CREATE | 128 | 70,081,478 | 0 |
| CREATE2 | 16 | 17,668,890 | 0 |
| CREATE2 | 128 | 149,639,402 | 0 |

At N16/L49152, zero→JUMPDEST changes actuals by+3,292,399 for CREATE and+3,288,681
for CREATE2, with zero extra node price and identical charged user gas. CREATE full
price is unchanged; CREATE2 full price decreases407,744 PGU due to changed trie
selectors. Thus neither full coverage nor conservative repetition pricing means
all marginal work is separately priced. CREATE2 pairs also change created addresses;
all their trie selectors and prices remain visible rather than attributed entirely
to bytecode analysis.

The synthetic short-initcode CREATE2/N128 full quote is3.6949× actual. This finite
conservative overestimate is recorded, not tuned away: lowering a wrapper from only
short/easy cases could remove the headroom that covers long/high-work initcode.
Retain this cohort as a regression constraint for any future wrapper refinement.
No Mainnet replay was repeated: quote binary/model are unchanged from the preceding
50-proposal run, whose retained2.146789704× historical result remains separate.

## Audit and next boundary

Final independent actual-campaign audit SHA256:
`abf246438c68381f194c30c9c3620d798d8c25ff815cb670384e0f4f3b67f339`.
It checks all input/native/quote/SP1 identities, raw normalized PGU, complete statuses,
nonce/opcode/preparation gates, exact rational ranks,24 paired comparisons, four
length DIDs and two content comparisons. All24 resource receipts name four objects
each; the96 objects were independently verified absent in the host namespace after
execution. Peak sampled process-tree PSS was1,778,855,936 bytes (about1.66GiB).
The SDK memory policy and host PSS guard remain distinct; no proof/GPU-throughput
claim follows from this execution-gas measurement.

This stage found no underpriced whole-proposal or repetition candidate at the tested
boundaries. It does not certify every legal composition. Further investigation should
focus on work that can repeat without an existing large per-event wrapper, while
keeping repeated initcode, rollback, content and nonempty output cases available for
future pricing changes. Gas-saturated capacity and production guest alignment remain
separate tasks.
