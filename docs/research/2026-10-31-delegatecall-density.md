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

## Actual successor and prospective validation

The generated candidate is
`2fb4bb71092f11cca2217e69b25e94217342ffb46a540a961e7a77a483cb5601`,
from reviewed Reth source `75737a8190e388312457f5cd5b111ee291688046`.
Actual native repricing of all 24 prior inputs confirms the exact scaled
increase of 2158093278940683091381 per confirmed F4 event. Unrelated selectors
and pricing terms, inputs and public outputs stay unchanged; derived totals
update by exactly this uplift. The development cohort
goes from one full deficit to zero, with all ten increments covered. The 16
checked reference observations remain covered, including all 20 increments;
they are known replays, not fresh measurements or holdouts. The original
STOP/N32768 failure now quotes 212,898,652 node and 367,867,019 complete PGU
against 311,589,746 unchanged actual PGU.

All six fresh PUSH0 inputs completed under the frozen candidate. Their old
parent quotes would leave one full deficit; the candidate covers all six
observations and all six within-terminal N0/N4096 increments. No prospective
actual was used to choose or adjust 6000.

| Child / calls | User EVM gas | Actual SP1 PGU | Candidate node zkGas | Candidate complete PGU |
| --- | ---: | ---: | ---: | ---: |
| STOP / 0 | 53,291 | 161,069,137 | 9,719,397 | 164,687,764 |
| STOP / 4096 | 530,987 | 179,654,669 | 35,093,966 | 190,062,333 |
| STOP / 65536 | 7,696,427 | 459,270,055 | 415,712,505 | 570,680,872 |
| RETURN / 0 | 54,091 | 161,069,496 | 9,723,400 | 164,691,767 |
| RETURN / 4096 | 556,363 | 180,030,742 | 52,082,232 | 207,050,599 |
| RETURN / 65536 | 8,090,443 | 465,280,865 | 687,464,715 | 842,433,081 |

Literal actual/node ranking retains startup-dominated N0 controls. Complete
quote/actual across the fresh cohort ranges from about 1.0225x to 1.8106x;
these are finite synthetic observations, not the historical Mainnet average.
Peak sampled process-tree PSS was 4,118,722,560 bytes (about 3.84GiB), below
the 6GiB guard. Independent host-namespace checks found all 32 development
and 24 prospective owned shared-memory objects absent after execution.

The independent actual audits are
`40ba8b660dc325f2d2a8a6f66c60dcfb359328ce5c2f5bf29aa242eb6b456499`
(development) and
`5b6565d8b2e83564b02296f8cc3c29ff69ffe0f6a731d9e4896dd2240fea11a5`
(prospective). Source/model/SDK identities, exact retained inputs and public
outputs, premeasurement policy/readiness ordering, and exact-scaled deltas
were checked independently. No guest or actual-SP1 rerun was needed to reprice
earlier inputs.

## Historical sensitivity

Native repricing of the same retained 50 Mainnet proposals / 9600 blocks changes
full quotes from 150,234,443,229 to 150,255,527,803 PGU: +0.0140344% overall,
maximum +0.0205618% for a proposal. Retained actual PGU totals 69,980,978,094;
weighted full/actual moves from 2.1467897x to 2.1470910x. Full deficits remain
0/50 before and after. All resources, selectors, Unzen observations, inputs,
public outputs and proposal-once prices remain unchanged. The 9770 F4 events
are inferred from the sole exact coefficient delta, not a separate historical
opcode trace.

This reuses historical SDK6.1 / ELF416f actuals, separate from the controlled
SDK6.3 / ELFb76 cohorts. It is not fresh daily collection, a new historical
SP1 run or strict current-program accuracy certification. Public historical
reports retain input/source identities; historical GuestInputs are not part
of this synthetic fixture. The original local cache was available during the
native run and checked against its source manifest; it was no longer available
for independent rehash afterward. The independent audit verified all 50 raw
report pairs and 9600 block deltas and is bound by
`b2f455b843a7ca102d10a54490b1ba996c8fba088d0e65c74bd39376c73a3b0c`.

## Tooling and source identity

The initial producer is `f5502f38` and the prospective producer is `9ff22c61`,
each with a distinct 495-file source inventory. The original constructor has
484 source identities; the unchanged experimental guest has 67. Candidate
quote `0b807b701f75206bc7f10e4161b00082f005b4c0e9e87948dd4ec1f837eb7d18`
uses the original 490-file CLI source with a documented temporary five-package
Reth override. Only its comparison-snapshot Cargo.lock changes; the effective
reviewed Reth source and model are recorded separately. Null revision/modified
build metadata is preserved. Committed dependency pins and guest artifacts were
not changed. Native, quote and sampler interfaces continue to consume the same
legal GuestInput. Reusable producer profiles, regression tests and usage remain
in Raiko; fixed data and reports belong in Atlas.
