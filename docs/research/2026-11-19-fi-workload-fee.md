# FI workload accounting and fee sensitivity

Status: offline research; no tariff, admission, guest, or deployed fee change.

## Decision and scope

Keep the current opcode/precompile/memory/resource prices fixed while evaluating Etna FI.
Account for work actually performed for a discarded FI singleton, including its first crossing
atomic operation and trial-only state/witness access. Evaluate FI enqueue fees as the primary
economic control. High fees and congestion-driven exponential increases are acceptable candidates;
they do not by themselves establish that every legal FI workload can be proved.

The user selected fee-first evaluation, with targeted zkGas corrections only if measurement
demonstrates a missing or materially underpriced operation. This study compares the existing linear
backlog shape with a doubling candidate. It does not change the protocol's eligibility, full-budget
singleton decision, one-entry-per-block consumption, or committed transaction fees.

## Evidence and current implementation boundary

- Review comment, 2026-11-18: [Etna FI metering feedback](https://app.notion.com/p/4169673143d68029b7e3c08055b3bfef?d=41c9673143d681f7843c001cd1ba1a73).
- [FI rules at `991e06f6`](https://github.com/taikoxyz/taiko-mono/blob/991e06f6d94842b2f57c67a4a8ba254b3c5a8830/packages/protocol/docs/etna/SPEC.md#L449-L505): an eligible due entry omitted from the body requires a completed singleton build from the committed parent with the same attributes and full block budgets. Revert/OOG inclusion differs from zk discard. Each block consumes at most one queue entry.
- [Resource acceptance at the same revision](https://github.com/taikoxyz/taiko-mono/blob/991e06f6d94842b2f57c67a4a8ba254b3c5a8830/packages/protocol/docs/etna/ACCEPTANCE.md#L5-L12) includes actual block work, the FI singleton and the whole first crossing operation, plus real legal proof combinations and sustained service.
- [Current quote at `4795e495`](https://github.com/taikoxyz/raiko2/blob/4795e495ba25d466aa338245e00b2ed1ee79f0bb/crates/zkgas-trace/src/runtime_quote.rs#L544-L618) combines attempted execution contributions with committed exact resource/MPT contributions. Its public entrypoint accepts Shasta GuestInput; it does not execute the independent Etna FI relation.
- The separate Etna proving implementation plan at `abc37e824c3d1aa1b00895d648c5f00a7d7b4c22` still marks P04 execution/FI and P07 real-backend acceptance as TODO. This is a dependency, not a measured zero-cost path.

The current deliverable is an offline fee/cost sensitivity calculator and a measurement contract.
It supplies no new SP1 observation, no Etna GuestInput, and no proof-completeness result. Existing
Shasta observations retain their original program, model and scope.

## Accounting contract for the Etna execution stage

Report three scopes separately, using the existing quote and observation identities when the
execution stage supplies them:

1. Actual block execution, preserving all attempted work performed on that path.
2. FI singleton trial from the same parent, attributes and full budgets. Distinguish transaction
   validation rejection, inclusion/success, included revert/OOG, zk discard and metering overflow.
3. Combined block and complete range. Attribute trial-only state reads/witness nodes and shared
   work according to actual execution; do not add two standalone startup/materialization charges
   when the combined program performs that work once. Conversely, do not remove work actually
   repeated by independent executions.

Retain the full first crossing atomic operation/precompile, not only its accepted prefix. Keep
normal cursor advancement, due/not-due and ineligible cases in the differential test matrix.
Failed, missing and incompatible evidence stays explicit. Do not allocate a proposal PGU total
to individual transactions or blocks. An FI-only delta requires a matched same-program control;
two independent full-program executions are a diagnostic until sharing is measured.

Freeze EL revision/rules/budgets, input and parent/attribute identities, model, guest ELF/VK,
SDK/backend and measurement stage. Start with SP1 for pricing diagnosis. Etna deployment
acceptance still requires the legal proof combinations specified by the protocol.

The initial execution matrix for P04 is finite:

| Case | Required distinction | Work to retain |
| --- | --- | --- |
| No pending FI / pending but not due | No consumption versus required queue witness | FI witness/authentication overhead where actually present |
| Due, ineligible bytes | Legal skip without singleton execution | Decode/eligibility/authentication cost; no invented EVM trial |
| Due, eligible, nonce/balance/intrinsic rejection | Completed empty singleton versus infrastructure failure | Validation and trial-only reads actually performed |
| At the exact zk budget | Inclusion versus budget equality | Full successful attempt, receipt and cursor |
| One unit beyond the budget | zk discard versus ordinary execution failure | Attempted prefix and complete crossing operation |
| Direct and nested expensive precompile crossing | Wrapper/body ownership and first atomic overshoot | Full dispatched work, including failed calls where executed |
| Ordinary revert and EVM OOG | Included failure still counts as FI inclusion | Executed failure work and retained receipt/fees |
| Trial reads absent from the actual body | Trial-only state/code/witness resources | Actual union and repeated materialization/query work |
| Trial and body revisit the same state/code | Actual sharing versus repeated computation | Observed sharing, not an assumed sum or free reuse |
| Repeated due FI across K blocks with ordinary load | One consumption per block and full singleton budgets | Combined range PGU, memory, latency and legal proof combinations |

Include explicit negative controls for incorrect parent/attributes, smaller local budgets,
incomplete execution, invalid witness and infrastructure timeout. None may become a valid skip.
Keep successful/failed observations and unavailable backend measurements visible. The matrix is
a pending acceptance contract, not a statement that these Etna cases have been executed.

## Fee candidates

Let `p = tail - head`, including entries consumed on L2 but not yet landed, `T > 0` be a backlog
threshold, and `F0 > 0` the base enqueue fee in wei. The existing study baseline is:

```text
linear(p) = floor(F0 * (T + p) / T)
```

The offline exponential candidate uses integer interpolation between doubling points:

```text
q, r = divmod(p, T)
doubling(p) = floor(F0 * 2^q * (T + r) / T)
```

Both candidates charge `F0` at zero backlog and `2*F0` at backlog `T`. The doubling candidate
charges `4*F0` at `2*T`, where the linear baseline charges `3*F0`. It is backlog-responsive;
it is not an implementation of EIP-1559's time/utilization base-fee update.

Use exact full-precision integer intermediates and floor-to-wei arithmetic for this comparison.
This is an explicitly chosen research arithmetic policy, not an assertion about unspecified
contract rounding or intermediate overflow. Report an out-of-uint256 fee as unavailable; never
wrap or silently cap it. Bound inputs and computation before exponentiation or iteration.
A complete batch total is an exact mathematical sum and may exceed uint256; it does not simulate
an Inbox balance/storage update. A batch that reaches an unavailable individual fee preserves
the priced prefix and exposes no purported complete total.

A batch calculation enqueues entries sequentially with no landing/head advancement: entry `i`
observes `p+i`. Ordinary L2 consumption does not lower this pending measure. Actual deployment
evaluation must additionally cover settlement lag, honest FI users and sustained arrivals/service.
No time-to-drain guarantee follows from these fee totals.

## Economics and decision rule

With explicit scenario inputs only:

```text
incremental_SP1_USD = FI_extra_PGU / 1e9 * USD_per_billion_PGU
required_fee_wei = ceil(incremental_SP1_USD * reserve_multiplier / ETH_USD * 1e18)
```

The fee recipient is the protocol/DAO, which separately funds operations under SPEC appendix C;
this is protocol revenue coverage, not an automatic payment to the submitting lander or prover.
L1 submission gas is another sender expense and does not become protocol fee revenue. The
calculator excludes unmeasured second-proof, landing, DA, validator and operator costs. Missing
costs must not be interpreted as zero or as full operational cost coverage.

- If the existing model tracks the measured FI work and the required fee is acceptable, tune FI
  fee parameters while preserving operation prices.
- If the model tracks work but fees/capacity are unacceptable, evaluate execution cost and capacity
  configuration with the unchanged protocol relation before changing prices.
- If measurement exposes a missing selector or material underestimate, fix that demonstrated
  pricing issue and regress existing normal/pressure inputs. A higher fee does not establish
  model accuracy or make an otherwise unprovable legal input provable.

The example uses the SPEC's illustrative `0.01 ETH` base fee, thresholds 50/200/1000, an explicit
hypothetical `2000 USD/ETH`, the user's illustrative `0.5 USD/billion PGU`, and a 1.5 reserve.
None is a production quote, current exchange rate, measured FI maximum or approved parameter.
The 3.6B and 15B workload rows are sensitivity assumptions, not Etna measurements or limits.

## Reproduction

From the repository root, using the existing Python environment:

```sh
~/.venv/bin/python bin/zkgas/bench/fi_fee_report.py \
  --config bin/zkgas/bench/examples/fi-fee-sensitivity.json \
  --output fi-fee-report.json
~/.venv/bin/python -m unittest discover \
  -s bin/zkgas/bench/tests -p 'test_fi_fee_report.py' -v
```

The report embeds the normalized assumptions and hashes the raw configuration and calculator
source. Output is deterministic and exclusively created: existing files are preserved. No RPC,
market data, prover, guest build or deployment is invoked. The
[retained report](data/2026-11-19-fi-fee-sensitivity.json) binds the checked-in example and
calculator source; the findings below describe that report.

## Sensitivity findings

These are exact arithmetic results under the example assumptions, independently calculated
from closed-form batch sums. They are not new observations of FI execution.

| Backlog threshold T | Linear fee for 900 entries (ETH) | Doubling fee for 900 entries (ETH) | Doubling fee of one entry at pending=300 (ETH) |
| --- | ---: | ---: | ---: |
| 50 | 89.91 | 195296.535 | 0.64 |
| 200 | 29.2275 | 64.885 | 0.03 |
| 1000 | 13.0455 | 13.0455 | 0.013 |

For `T=50`, the one-entry doubling fee at pending=900 is 2621.44 ETH. This candidate can make
sustained enqueues extremely expensive without increasing ordinary operation prices. The
threshold materially changes that outcome; it is not selected for production by this study.
Because pending includes consumed-but-unlanded entries, the pending=300 row can also describe
settlement lag with no remaining L2 execution backlog. That user-cost effect must be evaluated
before selecting the curve or threshold.

The zero-backlog fee remains 0.01 ETH under both curves. At the explicitly hypothetical
2000 USD/ETH it is 20 USD. The incremental SP1-only cost sensitivity is:

| Assumed extra FI PGU | SP1 cost with 1.5 reserve (USD) | Minimum fee for this cost component (ETH) |
| --- | ---: | ---: |
| 0.2B | 0.15 | 0.000075 |
| 1B | 0.75 | 0.000375 |
| 3.6B | 2.7 | 0.00135 |
| 15B | 11.25 | 0.005625 |

Thus even the example base fee covers these particular assumed SP1-only costs. This establishes
the economic parameter relationship, not that 15B is a legal/provable FI bound, that 0.01 ETH is
an appropriate user fee, or that the complete Etna operating costs are covered. The sparse-backlog
base-fee test is essential: a steep congestion curve cannot compensate for an underpriced first
expensive entry when the queue is empty.

No current tariff adjustment follows from these arithmetic results. The outstanding evidence is
the actual FI path and first-operation overshoot under pinned EL/guest rules, combined-program
resource sharing, complete legal proof costs and capacity, and honest-user cost during settlement
lag. Add immutable FI fixtures to Atlas when that execution evidence exists; keep calculators in
Raiko's composable tools.

## Verification

- The focused `test_fi_fee_report.py` suite passes all 9 tests, including real CLI complete/partial/
  invalid exits, exact units and rounding, checked individual-fee overflow, explicitly partial
  batches, and preservation of existing outputs.
- Independent code review found no material issue. A separate rational oracle checked 20,000 fee
  values; 64 concurrent publishers produced exactly one intact output with no temporary-file leaks.
- Independent behavioral verification exercised 92 fresh CLI runs, including strict-input and
  numeric boundaries, and regenerated the retained report byte-identically.
- The existing bench regression, `~/.venv/bin/python -m unittest discover
  -s bin/zkgas/bench/tests -v`, completed successfully: 622 tests discovered, 551 passed and
  71 optional native/compiler checks skipped. This includes the 9 new focused tests.
- The retained report SHA256 is `6dc2de6e852d3e709f524208ebca03cdab6f45910eeeb8894cc12e45a4af2d58`.
  Its calculator/config hashes are verified against the checked-in files.
- Path hygiene and local documentation links pass. No runtime, tariff, ELF/VK, guest or dependency
  file is changed. These checks validate the research calculator, not Etna execution or proving.
