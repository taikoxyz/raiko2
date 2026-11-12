# Near-30M gas composition under frozen pricing

The first gas-budget composition panel completed32 new one-block proposals:
16 own-N0 controls and16 full-work inputs. Every full input consumed at least
99% of the30M charged whole-block EVM gas budget. No measured node shortfall
exceeded the [2B priority threshold](../zkgas-evaluation-standard.md#gas-bounded-priority-and-pricing-complexity).
This panel supplies actual saturation measurements following the earlier
[storage projections](2026-11-02-gas-bounded-pricing-priority.md); it does not
establish a bound for untested combinations.

## Measurement and retained quantities

For each canonical GuestInput, retain three separate quantities:

- Actual identical-proposal SP1 PGU, normalized as
  `execution_report.gas * 10 // 191` using SDK6.3 execute-only measurement.
- Every block's node-observable estimate and their literal sum. These inputs
  each contain one block, so the sum equals that block's quote.
- Full proposal estimate, including the proposal/input terms unavailable to the
  node's block-local quote.

Node shortfall is `max(actual - literal_node_sum, 0)`; full-proposal shortfall
is calculated separately. Own-N0 differences diagnose work increments while
preserving the absolute observations. The controls, startup cost and their
underpricing flags remain in the dataset.

The source was committed before actual sampling at Raiko revision
`066019d456926133ad6e1033fccc3e1dbc30fba9`. Model
`c8936124c1b074b57abf36873e1c37e01f9bc49fa6c9bb161a92542f1e6551ef`,
SDK6.3 and ELF SHA256
`b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`
were frozen. The panel did not fit coefficients or change pricing, SDK, guest,
node admission policy or the block gas header. The retained ELF/source binding is
provenance, not a build attestation
or production-program parity claim. Measurement executes the guest; it does not
produce a proof or measure GPU proving time.

The explicit experimental execution profile is `evm-gas-only`. The legacy
Unzen pricing cap is disabled for these experiments. No replacement node limit
was selected, so acceptance by a future node pricing limit remains unknown.
The unchanged native block header has31M gas;30M is the **charged research
budget**, including the fixture's167,355-gas Anchor. Transaction declaration,
gross execution, refund and final receipt gas are checked separately. These
inputs are not evidence of admission under a30M header.

## Programs and qualification

The density panel contains20 inputs: five patterns, packing4 or16 user CALLs,
and own-N0/full pairs. Patterns are JUMPDEST, PUSH0/POP, DUP16/POP, paired SWAP16,
and mixed JUMPDEST/DUP16/POP. The2048-site body is identical within each pair.
Distinct stack seeds and a separate memory counter have checked final values.
The fixed-body instruction census, gas formulas and ordered operand checks are
retained in the producer's tests. Full native execution verifies every input's
success, counters and exact opcode totals.

The journal panel contains12 inputs: dirty/direct, dirty/caught-REVERT and
dirty+hash/caught-REVERT, with the same4/16 packing and own-N0/full pairing.
Each user starts from original-zero storage, seeds5, cycles7/11, and restores
the original value. The mixed body hashes a zero memory word between writes.
The parent verifies the caught child's status,32-byte return length, exact
loop-derived marker and rolled-back storage. Storage access is prewarmed;
cold-first-child rollback is outside this panel.

Qualification includes9 native-only negative candidates. Four retain successful
receipts despite wrong operand/state semantics and are caught by the independent
oracle; five are native rejects. They have no actual PGU measurement and are
kept outside the32 measured inputs. Native success and opcode counts alone
are insufficient semantic checks.

Two pre-actual revisions are retained as diagnostics:

- The density loop body grew from128 to2048 sites before actual sampling. This
  reduces loop-control JUMPDEST gas from approximately30% to2.62% for the
  JUMPDEST body. Earlier native screenings have no SP1 actuals.
- Journal gas declarations initially used gross+1024. Actual native failures
  exposed the SSTORE gas-left sentry. The final declarations retain2301 gas
  above gross and, for children, EIP-150 headroom. Code, batches and predicted
  charged work were unchanged by that envelope correction. The failing envelope
  is retained as a negative; it was not a pricing failure.

## Actual results

All32 actual measurements completed once, with no failed or censored SP1 attempt.
The following table contains the16 full-work observations. Values are literal
PGU; gas includes the Anchor and all user receipts.

| Pattern | Users | Charged block gas | Actual PGU | Node estimate | Full estimate | Node shortfall |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| DUP16/POP |4|29,985,283|803,381,722|1,039,557,769|1,194,499,600|0|
| DUP16/POP |16|29,831,387|796,791,108|1,028,073,671|1,183,015,502|0|
| JUMPDEST+DUP16/POP |4|29,975,267|923,318,167|1,247,940,687|1,402,882,517|0|
| JUMPDEST+DUP16/POP |16|29,936,939|918,036,042|1,238,385,251|1,393,327,081|0|
| JUMPDEST |4|29,989,743|1,516,692,616|2,276,972,238|2,431,914,068|0|
| JUMPDEST |16|29,986,875|1,507,080,896|2,259,874,248|2,414,816,079|0|
| PUSH0/POP |4|29,975,511|954,226,871|1,265,881,415|1,420,823,245|0|
| PUSH0/POP |16|29,936,523|948,605,641|1,256,118,044|1,411,059,875|0|
| SWAP16/SWAP16 |4|29,975,267|1,896,374,155|2,226,715,590|2,381,657,421|0|
| SWAP16/SWAP16 |16|29,936,939|1,881,507,050|2,207,403,616|2,362,345,447|0|
| Dirty/direct |4|29,962,343|613,479,154|690,929,066|845,871,153|0|
| Dirty/direct |16|29,980,427|584,931,928|687,637,309|842,579,396|0|
| Dirty/REVERT |4|29,963,603|831,100,908|711,005,659|865,947,745|120,095,249|
| Dirty/REVERT |16|29,985,467|795,282,987|767,912,064|922,854,151|27,370,923|
| Dirty+hash/REVERT |4|29,987,175|1,025,890,650|963,317,734|1,118,259,821|62,572,916|
| Dirty+hash/REVERT |16|29,755,451|990,689,306|1,009,932,495|1,164,874,582|0|

Across all32 inputs, node underpricing remains19/32: all16 controls and the
three full inputs shown with positive gaps. Full proposal underpricing is0/32.
The largest measured node shortfall is153,314,777 PGU in the four-user JUMPDEST
control; the full-work maximum is120,095,249 PGU in four-user dirty REVERT.
Maximum actual workload is1,896,374,155 PGU, a different observation from either
maximum shortfall. All16 matched actual work increments are covered by both
the corresponding node and full quote increments, using scaled quote arithmetic
before rounding. This does not erase any absolute shortfall.

The older one-user dirty-REVERT observation remains valid. This new panel changes
transaction packing, per-user iteration count and declaration context. New
covered increments do not show that its earlier8.086M PGU increment gap was
fixed. No tariff changed. The contribution of reuse across users has not been
isolated and remains an attribution TODO.

Sampling was sequential with a6GiB process-tree PSS guard,600-second timeout
per sample and bounded output/log sizes. Peak sampled process-tree PSS was
4,570,995,712 bytes. All128 distinct owned shared-memory objects were confirmed
absent after sampling; resource receipts remain retained. These guards constrain
the experiment, not a production prover capacity claim.

## Decision and remaining coverage

This panel found no observed greater-than2B node shortfall within30M charged
gas. Under the agreed [complexity tradeoff](2026-11-02-gas-bounded-pricing-priority.md),
its residual journal/startup discrepancies stay low priority in the tested
domain. No additional state instrumentation or tariff branch is justified by
these results alone. The32 observations cannot establish a maximum, arbitrary
transaction composition coverage or a production proving deadline.

Mixed bodies here combine opcode families **within one runtime**; each block's
users still repeat the same pattern. Next coverage should include heterogeneous
transaction order and cross-family combinations, plus distinct accounts/cold
access and rollback topology. Compare their complete-block shortfall against
the same30M/2B policy. Do not add safe per-family labels to infer that an
untested mixture is safe. Retrospective simplification remains possible through
a separately measured candidate, without rewriting these frozen results.

## Maintained tools and verification

The generators, semantic/gas tests and read-only priority report live under
[`bin/zkgas/bench`](../../bin/zkgas/bench/README.md#near30m-charged-gas-density-composition).
Canonical requests/GuestInputs, raw native/quote/SP1 reports, controls, native
negatives, model/program identities and independent audits belong to Atlas
fixture [`unzen-gas-budget-composition-v1`](https://github.com/taikoxyz/raiko2-atlas/pull/25),
with an [immutable data Release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-gas-budget-composition-v1).
Atlas contains data; the Raiko source
owns execution and reporting. An archived projection of operator receipts is
not a resumable workload directory; use its canonical GuestInputs in a fresh
output root to repeat execution.

Checks actually completed before the results record:

- `~/.venv/bin/python -S -m unittest discover -s bin/zkgas/bench/tests -v`:
  187 tests discovered,153 executed successfully and34 optional tests skipped.
- Independent native qualification for all32 final inputs and9 native-only
  negative candidates; no guest or Rust rebuild.
- All32 bounded SP1 execute-only measurements; independent audit of input,
  output, receipt gas, program/model identities, normalization and resource
  cleanup.
- Independent recomputation of all32 report rows and16 paired increments;
  source review and `git diff --check` passed.
- Atlas independent data/metadata/archive review passed. The696-member archive
  passed compressed/decoded and inner checksum closure, followed by the unchanged
  boundary-fixture workflow against a fresh published GitHub download. A
  temporary-index combined tree with Atlas #22/#23/#24 verified all four
  registrations and metadata checksum sets without altering real refs or index.

Optional native/backend tests are not implicitly covered by their stdlib skips;
the separate32-input qualification and actual audit provide the listed backend
evidence. This record makes no fresh claim about unrelated workspace CI lanes.
