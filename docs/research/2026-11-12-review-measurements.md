# Common-price and completion review measurements

This is a development checkpoint following the external model review. It does not replace
the [frozen-model capacity report](2026-11-11-frozen-model-capacity.md), certify admission,
or change an accepted price. Raw Unzen retains its own score unit throughout.

## Fixed identities and population

The numerical predecessor is
`80160649d7d29a85879cd9524b35c271a4b8638f64de94eef91d016e56121cf8`.
Actuals use research ELF
`b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`
and SP1 SDK 6.3.0, execute-only. PGU is `floor(raw ExecutionReport.gas * 10 / 191)`.
No guest or SDK was rebuilt for this experiment.

The producer is [`ordinary_placeholders.py`](../../bin/zkgas/bench/cases/ordinary_placeholders.py).
It freezes 23 requests: eight ORIGIN, six CALL, six caught normal REVERT and three reused
ECRECOVER requests. All are synthetic development probes. Target and scaffold lanes retain
their own zero controls. Native qualification checks literal bytecode, status, full gas,
opcode counts and child returndata; sample names alone are not coverage evidence.

The 18 non-reserved ORIGIN/CALL/REVERT inputs were each measured once: 18 unique inputs,
18 successful SP1 attempts and no sampler failures or retries. Three ECRECOVER observations
were reused after checking their original input, public output, ELF, SDK and actual hashes.
The two ORIGIN720 observations were still unopened at this checkpoint. Native qualification
also retained one constructor schema rejection before using the compatible constructor;
the entire preparation campaign must not be described as having zero failures.

Quote producer SHA-256 is
`066b970929c3ef3247c2bd68dd8679187da8c49fbcc970bb353fb6a6cc9511a7`.
This is the retained predecessor build, including its original dirty-build identity, not
a newly relabeled executable from the review-fix revision. Native constructors remain
separately pinned in the qualification receipt. The prepared catalog SHA-256 is
`00fd67dbabec0026e4ad8d3750250b0a6c0e4d73d6ef645465e7cadfe829615a`;
qualification receipt SHA-256 is
`834eeb3bc3e6ea0a05dd1fb417be7de9fc13d8a26e0e56d7138c88b046c57c97`.

## Whole-input observations

Actual and quote columns are whole-proposal PGU. EVM gas is charged user-transaction gas,
not a PGU conversion. The node column is the literal block-known quote, without startup
or control subtraction.

| Target | Count | EVM gas | Actual | Node | Full |
| --- | ---: | ---: | ---: | ---: | ---: |
| ORIGIN | 0 | 21,049 | 161,032,931 | 7,615,938 | 162,557,769 |
| ORIGIN | 1 | 21,112 | 161,034,619 | 12,618,898 | 167,560,728 |
| ORIGIN | 8 | 21,553 | 161,046,667 | 47,639,615 | 202,581,446 |
| CALL | 0 | 23,716 | 161,101,132 | 8,646,055 | 163,623,398 |
| CALL | 1 | 23,923 | 161,109,028 | 9,659,891 | 164,637,233 |
| CALL | 8 | 25,372 | 161,162,889 | 16,756,738 | 171,734,081 |
| Caught REVERT, including CALL | 0 | 23,722 | 161,092,538 | 13,646,260 | 168,623,603 |
| Caught REVERT, including CALL | 1 | 23,935 | 161,106,654 | 19,660,300 | 174,637,643 |
| Caught REVERT, including CALL | 8 | 25,426 | 161,160,398 | 61,758,583 | 216,735,926 |

For N1 and N8, same-target differences from N0 per added iteration are respectively:

| Target | N1 increment | N8 mean increment |
| --- | ---: | ---: |
| ORIGIN | 1,688 | 1,717 |
| CALL | 7,896 | 7,719.625 |
| Caught REVERT, including CALL | 14,116 | 8,482.5 |
| Reused valid ECRECOVER | 72,805 | 68,066.875 |

These differences include loops and guards. They are not isolated opcode actuals, and no
scaffold credit is applied. They support investigating the normal-route placeholders:
ORIGIN's quote increment is about 5M, warm empty CALL's about 1M, and a CALL with empty
normal REVERT about 6M. They do not justify lowering every related route to these observed
increments.

The candidate investigation first isolates normal ORIGIN. The largest exact slope across
all three target pairs is `12048/7`. Applying 25% margin and rounding upward to a multiple
of 64 proposes 2,176 PGU for the normal fixed body. Feature-error pricing and every other
route remain separate. Candidate derivation, rendering and reserved observations must be
checked before adoption; this report does not state that adoption has occurred.

CALL requires additional cold/value/nonempty-input gates. Normal REVERT requires a
preallocated, nonempty large-returndata gate: the current normal route can have zero raw
gas for both empty and large preallocated memory, so the empty-case result cannot safely
price the latter. ECRECOVER currently selects other candidates as well as its fixed floor;
lowering only that floor is not an isolated change to its effective price. One valid vector
also does not establish the full operand envelope.

All 18 literal node deficits remain visible, with a maximum of 153,417,031 PGU; full
deficits are zero in this development subset. Startup or scaffold subtraction must not
be used to make the node deficits disappear. The two ORIGIN720 controls remain a separate
constructed availability question, rather than evidence of historical transaction rejection.

## Proposal 45309 completion attribution

The retained Mainnet input is
`45cfddb6df4ff7ab7022fb1445f598087717b9f5ff2a2ec5d80f4abcc2a6f1ab`,
with 192 blocks. Its actual is 1,081,006,163 PGU under the same b76/SDK6.3 program;
the original actual-report SHA-256 is
`0de55442a2db8e6e571374d3e255eb5eb9743b93d250ba8d021a95a5b5454587`.
The numerical801 quote is retained evidence from the predecessor producer, not a fresh
measurement at the review-fix head.

| Quote component | PGU |
| --- | ---: |
| Literal sum of rounded node block quotes | 1,029,793,035 |
| Prove block aggregate, rounded once | 2,214,981,641 |
| Block-MPT scaled contribution, before final rounding | 1,185,188,648 |
| Proposal-once contribution, before final rounding | 160,605,710.735480402119958866 |
| Full proposal quote | 2,375,587,352 |
| Full minus literal node | 1,345,794,317 |

The proposal-once term includes startup 154,263,870.735480402119958866 and parent headers
6,341,840. In this retained honest input, the node/prove execution and resource selectors
otherwise agree: block MPT accounts for the block-level completion difference. Small
closure differences follow from summing rounded blocks versus rounding the scaled aggregate.
Use integer fixed-point arithmetic, not floating-point JSON conversions, to reproduce it.

This identifies which **quoted** terms dominate the overquote. It does not measure actual
MPT cost. The retained actual report has empty phase cycle/invocation trackers, so it cannot
prove whether execution is underpriced, completion is overpriced, or the totals partly
offset. Near equality between node and actual alone is insufficient reason to delete MPT
completion. Phase evidence and broader input variation remain necessary for repricing.

## Verification and remaining scope

Independent verification joined all 18 canonical inputs, native reports, quotes, actuals,
public outputs and program identities, and rechecked the three reused observations. Maximum
descendant PSS was 1,204,345,856 bytes; total SP1 wall time was 93.212 seconds, with a
maximum attempt of 5.314 seconds. The sampler retained its 6GiB PSS ceiling and exact owned
resource receipts. All 72 receipt-owned shared-memory names were absent after completion.

The generic patched-Unzen comparison is a separate experiment. Its coefficients and
whole-input conversion must be frozen before evaluation. A simple baseline is allowed to
win: neither more features nor these local placeholder findings establish superior global
accuracy. Previously opened panels remain development regressions, not untouched holdout.
No numerical tariff, production policy, fee, SDK, guest artifact, runtime mode or deployment
is changed by the observations reported here.
