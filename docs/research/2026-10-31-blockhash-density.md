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
BLOCKHASH checks the expected nonzero/zero result and warms the target. Then
a fixed body repeats DUP2/BLOCKHASH/POP 64 times per batch; only batch-loop
checks repeat. N0 retains the unused body and the initial semantic query.
Thus total BLOCKHASH events are N+1, including at N0. Count immediates use
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

All12 measurements completed using the same canonical inputs accepted by
independent native qualification. A second independent audit verified raw
SDK6.3 normalization, source/input/output bindings, all15 matched increments
and host-namespace cleanup of all48 owned objects. Peak measured PSS was
2,229,408,768 bytes. No backend run was repeated and no guest was rebuilt.

| Mode, bulk queries | User EVM gas | Actual SP1 PGU | Node-known PGU | Complete quote PGU |
| --- | ---: | ---: | ---: | ---: |
| Nearest,262144 | 6,738,518 | 302,104,066 | 30,872,210 | 185,814,041 |
| Farthest,262144 | 6,738,518 | 302,104,188 | 30,872,210 | 185,814,041 |
| Expired,262144 | 6,738,521 | 217,195,181 | 30,872,313 | 185,814,143 |

Six of12 complete quotes underpriced actual work; all15 within-mode N0/N64
increments underpriced. This exposes a repetition deficit masked by startup
in small samples. Nearest and farthest valid queries have similar workload;
the expired path is cheaper but also underpriced. These are whole-proposal
measurements, not isolated-opcode measurements.

Freeze a single-opcode rule before fresh validation: for each matched pair,
require its total node-known increment to cover1.25 times actual increment.
Select the maximum required BLOCKHASH coefficient with other charges fixed,
then round upward to100PGU. The threshold is612.5496654PGU, selecting700PGU
instead of32.117684584888432025. The exact rule and12row source summary are
bound by policy SHA256
`0b3878b9fa538baa871af449510a126bdf9b52b265f09e92e038a28e7ee1ec4e`.
No other coefficient, discount or limit changes are part of this correction.

Before any fresh actuals, define six prospective inputs: farthest/expired,
0/8192/196608 bulk queries, with one executed PUSH0/POP prelude in every
runtime. Even N0 has new canonical bytes and still one semantic BLOCKHASH.
They are excluded from selecting700; exact native counts and quote bindings
must pass before sampling. Candidate and validation results are pending.
