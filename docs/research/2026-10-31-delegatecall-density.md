# DELEGATECALL density without per-call guards

## Question

The [checked repetition cohort](2026-10-30-delegatecall-repetition.md) covers
its composite loop/guard/child work. Its STOP increment costs about 6,930 node
PGU per iteration versus 6,381 actual PGU, while the confirmed-call wrapper
alone costs about 3,842 PGU. Conservatively priced parent instructions could
mask a lower-priced wrapper. This follow-up removes that confounder without
claiming underpricing before measuring.

## Construction and completion

Eight original cases: child STOP or RETURN(0,0), total calls 0, 64, 1024 or
32768. Every parent has the same fixed 64-call body within its terminal series,
including N0. A fixed-width batch-count immediate selects 0, 1, 16 or 512 loop
iterations. A real CREATE deploys one tiny child before the loop, even at N0.
Each repeated call pushes zero input/output arguments, the actual child address
and 64 forwarded gas, executes DELEGATECALL and POP. There is one batch-loop
check; there are no per-call status or return-data checks in the EVM program.
The CREATE-address guard remains. The parent terminates with STOP.

Qualification must independently decode parent and child bytes and require exact
completed CREATE/call/child-terminal maps, zero exceptional instructions, correct
deployed runtime/address, factory nonce1 and successful user receipt. A successful
outer receipt alone is insufficient. Child code establishes empty return data;
gas headroom excludes EIP-150 truncation. Reject short batches, a wrong/empty
target, parent OOG and zero-forwarded-gas RETURN. Zero-gas STOP is valid.

Use one block, one CALL user, empty calldata, a 15M user gas envelope and the
explicit evm-gas-only profile. No extra child-account/code witness padding.
Preserve all eight controls/candidates and every rejection/failure. The earlier
16 checked requests must remain byte-identical under the default producer API.

## Measurement and evidence

Rank literal actual whole-proposal SP1 PGU / node-known zkGas. Complete quotes
and within-terminal N0/N64 repetition increments are separate diagnostics. Do
not subtract an assumed startup cost or assign actual PGU to individual frames.
Reuse b049 native, 0ead quote, 1998 streaming sampler, b76 ELF, SDK6.3 and model
a298 with their original source identities. No Rust/guest build or pricing
change is part of the initial measurement. A measured deficit requires analysis
before a separate pricing correction; historical impact must be evaluated if
prices change. Controls are finite model-development observations, not a global
node-admission bound, maximum block capacity or proving-throughput result.

## Initial measurement and frozen correction policy

All eight observations completed under parent model a298. STOP/N32768 measures
311,589,746 actual PGU, 142,182,252 node-known zkGas and 297,150,618 complete
quoted PGU: a 14,439,128 integer-PGU deficit. All five STOP N0/N64 matched
increments are underpriced; all five RETURN increments are covered. This is
evidence about the composite call/child/remaining-loop workload, not an isolated
frame coefficient. Independent evidence audit verified original input/output,
SDK PGU, source identities and cleanup of all 32 owned shared-memory objects.

Freeze the confirmed-DELEGATECALL fixed price at 6000 PGU/event before opening
new actual observations. The deterministic rule takes the maximum over the ten
same-terminal matched increments of

```text
old_wrapper + (5/4 * actual_increment - old_node_increment) / extra_calls
```

and rounds upward to a 1000-PGU quantum. The measured requirement is about
5541.90; rounding gives 6000. Keep every other route, memory coefficient, floor,
selector and global discount unchanged. Coverage projected from this formula
requires confirmation by real successor native quotes. The preserved parent
quotes must continue to show the original deficit.

Prospective validation uses six fresh PUSH0-argument inputs: STOP/empty RETURN
at N0/N4096/N65536. Preserve all earlier 24 requests byte-for-byte. Native
qualification precedes actual sampling; candidate tariff selection uses none
of these prospective actuals. Evaluate retained historical Mainnet proposals
with the new quote binary to quantify impact before describing a final result.
