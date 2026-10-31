# BLOCKHASH density

## Question and frozen scope

BLOCKHASH has a fixed price of about 32.118 PGU/event in model2fb. Source
inspection found indexed ancestor hashes and a 256-slot cache, not repeated
linear header scans. Constant-time lookup does not establish conservative
numerical pricing. Test high-density warm queries without repeated EVM guards.

Use unchanged nativeb049, quote0b807/model2fb, sampler1998/SDK6.3 and ELFb76.
Do not rebuild guests, change prices or address the unrelated RISC0 CI gate
in this initial measurement. Rank literal actual proposal PGU/node-known zkGas;
complete quotes and matched increments are separate diagnostics.

## Twelve-case construction

Nearest valid height B-1, farthest valid B-256, and expired B-257 each use bulk
counts 0/64/4096/262144. Compute the target from NUMBER once. One initial
BLOCKHASH checks the expected nonzero/zero result. Valid targets are already
warm after Anchor; expired targets return zero before host/database lookup. Then
a fixed body repeats DUP2/BLOCKHASH/POP 64 times per batch; only batch-loop
checks repeat. N0 retains the unused body and the initial semantic query.
Thus user-program BLOCKHASH events are N+1, including at N0. Count immediates use
fixed widths. One user CALL, empty calldata, one block, 15M EVM gas and the
explicit evm-gas-only profile; no child deployment or extra witness padding.

Independent qualification must decode actual stack order/target arithmetic,
64 static sites plus one semantic site, exact executed opcode maps, user
receipt/terminal and canonical input/public output. Validate the source
ancestor list is contiguous and the requested valid heights exist. Current
and future heights returning zero are extra native-only controls. Reject
short execution, wrong semantic expectations and parent OOG. Preserve all
controls/rejections without assigning native-only probes SP1 PGU.

Within-mode N0/N64 references isolate repetition. Cross-mode differences can
include semantic checks and target arithmetic; do not present them as pure
lookup deltas or assign isolated opcode actuals. A discovered deficit requires
separate frozen-policy correction and fresh validation. No global coverage,
throughput, maximum-capacity or production-admission claim follows.

## Initial actual results and frozen correction rule

All 12 measurements completed using the same canonical inputs accepted by
independent native qualification. A second independent audit verified raw
SDK6.3 normalization, source/input/output bindings, all 15 matched increments
and host-namespace cleanup of all 48 owned objects. Peak measured PSS was
2,229,408,768 bytes. No backend run was repeated and no guest was rebuilt.

| Mode, bulk queries | User EVM gas | Actual SP1 PGU | Node-known PGU | Complete quote PGU |
| --- | ---: | ---: | ---: | ---: |
| Nearest,262144 | 6,738,518 | 302,104,066 | 30,872,210 | 185,814,041 |
| Farthest,262144 | 6,738,518 | 302,104,188 | 30,872,210 | 185,814,041 |
| Expired,262144 | 6,738,521 | 217,195,181 | 30,872,313 | 185,814,143 |

Six of12 complete quotes underpriced actual work; all 15 within-mode N0/N64
increments underpriced. This exposes a repetition deficit masked by startup
in small samples. Nearest and farthest valid queries have similar workload;
the expired path is cheaper but also underpriced. These are whole-proposal
measurements, not isolated-opcode measurements.

Freeze a single-opcode rule before fresh validation: for each matched pair,
require its total node-known increment to cover 1.25 times actual increment.
Select the maximum required BLOCKHASH coefficient with other charges fixed,
then round upward to 100PGU. The threshold is 612.5496654PGU, selecting 700PGU
instead of 32.117684584888432025. The exact rule and12row source summary are
bound by policy SHA256
`0b3878b9fa538baa871af449510a126bdf9b52b265f09e92e038a28e7ee1ec4e`.
No other coefficient, discount or limit changes are part of this correction.

Before any fresh actuals, define six prospective inputs: farthest/expired,
0/8192/196608 bulk queries, with one executed PUSH0/POP prelude in every
runtime. Even N0 has new canonical bytes and still one semantic BLOCKHASH.
They are excluded from selecting 700; exact native counts and quote bindings
must pass before sampling. The candidate and validation results are recorded below.


## Actual successor replay and prospective validation

Candidate model `bbce9be0…` changes only normal BLOCKHASH to700PGU.
Host-only quote binary `8624f630…` uses the original490-file CLI snapshot
with five explicit local dependency overrides to reviewed Reth `35864b4e…`.
This comparison build changes no committed production dependency or guest.
All 18 original canonical inputs were actually repriced. The 12 development
full underquotes fall from 6 to 0; six prospective actuals also have 0 complete
underquotes and all 6 same-mode N0/N8192 increments are covered. No prospective
actual was used to select700; no old SP1 run was repeated for repricing.

| Case | Actual SP1 PGU | Parent complete PGU | Candidate complete PGU |
| --- | ---: | ---: | ---: |
| Farthest,262144,development | 302,104,188 | 185,814,041 | 361,067,696 |
| Farthest,196608,prospective | 266,827,920 | 179,731,800 | 311,215,120 |
| Expired,196608,prospective | 203,146,003 | 179,731,903 | 311,215,222 |

Prospective PSS peaked at1,985,410,048 bytes. These are execute-only SDK6.3
PGU measurements, not proof throughput. The maximum literal actual/node ratio
is still the small startup control, because node pricing excludes some
proposal/prover-only work; it is distinct from scalable repetition deficits.

### User counts, Anchor counts and projection scope

The native program observer deliberately begins after the Anchor transaction.
Its BLOCKHASH map therefore counts user N+1 only. The complete block price
also includes257 Anchor events for this embedded Anchor artifact at
B=1166000:255 iterations overB-2..B-256, oneB-1 in `_calcAncestorsHash`,
and anotherB-1 in `anchorV4`. Native candidate repricing confirms total
N+258 in every measured case. Other quote terms, Unzen scores, state, inputs
and public outputs stay exactly equal. This257 is fixture-specific; do not
assume it for other forks, Anchor artifacts or small block heights.

Anchor has already read all 256 valid ancestor hashes before the user call.
Thus valid-target user repetition measures warm cache access, including before
the semantic query; expired queries return zero before the lookup and do not
access or warm the cache. This does not isolate cold first reads. The source exporter's
initial underquote projection adds only user-event uplift to the parent
complete price. That is a conservative lower-bound projection, not the exact
full-block successor quote. Actual native replay includes Anchor uplift.
All same-mode paired increments cancel the fixed257 Anchor events, so the
frozen700 coefficient and25% margin rule are unaffected.

Retain the distinction in future work: varied/distinct target patterns,
cold-context paths in other block/Anchor settings, fork boundaries and more
complex opcode combinations remain finite future tests. No universal maximum
workload, gas-saturated capacity or mainnet activation claim follows here.


## Historical mainnet sensitivity

Replay the same 50 retained mainnet proposals (9600 blocks), revalidating all
canonical inputs against immutable Atlas archive `a3183011…`. Native old/new
quotes differ only by actual BLOCKHASH event count times the price delta;
state, all other quote components, Unzen scores, input hashes and public
outputs remain identical. Aggregate complete quote rises from150,255,527,803
to151,903,327,053PGU, or1.0966646%; the largest proposal increase is1.6265893%.
Complete underquotes remain0/50. The weighted complete/actual ratio moves
from2.147091 to2.170637; this preserves the earlier roughly 2x sensitivity
conclusion rather than materially changing normal historical pricing.

The actual workload sum69,980,978,094PGU remains the original SDK6.1/ELF
`416f5a47…` observations. These are historical cross-program sensitivity
diagnostics, not fresh SDK6.3/b76 actual measurements or strict accuracy
certification for a current production ELF. No historical SP1 execution was
repeated; candidate native replays and synthetic same-program actuals are
reported separately.
