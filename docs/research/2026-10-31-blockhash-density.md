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
