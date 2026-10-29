# Preallocated CALL return-data copying

## Question and construction

This checkpoint tests automatic CALL output copying after the parent has already allocated
its input and output memory. The earlier spawn-memory checkpoint measured allocation with
an empty child result; it did not isolate transporting actual returned bytes.

`bin/zkgas/bench/cases/return_copy.py` compiles one `ReturnCopy.sol` runtime using an explicitly
selected Solidity 0.8.21 compiler. The base profile has 90 cases and the optional repetition
profile has 12. Each zero/half/full triplet changes only the output-capacity word of a fixed
256-byte transaction input. The parent reserves the same distinct input/output ranges and
guards before the loop. Child RETURN and caught REVERT cover all four CALL variants;
IDENTITY supplies composition controls. No RETURNDATACOPY instruction is used.

The runtime checks call status, complete return-data length, an alternating first byte,
the copied boundary, uncopied sentinels and allocation guards. Native screening additionally
requires exact completed CALL/RETURN/REVERT counts, including reverted child work. This
distinguishes a valid zero-length REVERT from an exceptional halt with empty return data.

The objective remains actual proposal SP1 PGU divided by node-known zkGas. A complete quote
and paired copy/repetition deltas are separate diagnostics; proposal startup is not a node
charge. An unpriced marginal increment alone does not establish a scalable total-cost gap.

## Admission and qualification

All 102 plans are retained: 84 pass native construction and 18 are rejected. All 28 accepted
triplets have identical complete native opcode maps and equal node and complete quotes.
Additional negative verification deliberately requested an incorrect REVERT count and was
correctly rejected.

The rejected groups have sufficient source-derived bounds above the existing 100M Unzen
admission cap:

- For the 1 MiB cases, the parent allocation reaches 65,557 memory words. Its memory-cost
  increase after the initial 16 words is 8,590,607 EVM gas. The MSTORE multiplier of 29 alone
  makes this 249,127,603 Unzen units, before loop, child and Anchor work.
- For 512 repeated 64 KiB delegate calls, each spawn costs 59,500 Unzen units. Fresh leaf
  memory costs 14,336 EVM gas, with a minimum applicable multiplier of 10. Together these
  give at least 103,864,320 Unzen units before parent, loop and Anchor work.

The raw constructor logs only report that the expected Anchor and user transaction pair
was not committed. These bounds explain why these plans cannot pass the current cap;
they do not identify an exact halt instruction or establish EVM out-of-gas. Rejected plans
have no actual SP1 PGU measurement. They also remain an admission limitation of this finite
fixture campaign, rather than evidence about every block a future pricing configuration
might admit.

## Measured result and pricing decision

All 84 accepted inputs completed actual SP1 execution; there are no pending measurements or
quote/SP1/integrity failures. Complete quotes cover all 84 observations. The largest
actual/complete-quote ratio is 0.982553877. All 21 matched repetition increments are covered
by the corresponding node-price increments; the largest incremental actual/node ratio is
0.419919134.

Automatic copying does measurable work while the triplet's price remains constant. At
64 KiB and 256 successful DELEGATECALLs:

| Output capacity | Charged user EVM gas | Actual proposal SP1 PGU | Node-known PGU | Complete quote PGU |
| --- | ---: | ---: | ---: | ---: |
| Zero | 4,225,657 | 179,225,756 | 63,907,839 | 218,849,669 |
| Half | 4,225,669 | 181,506,163 | 63,907,839 | 218,849,669 |
| Full | 4,225,669 | 183,777,182 | 63,907,839 | 218,849,669 |

Full copying adds 4,551,426 actual PGU. The 12-gas transaction difference is the intrinsic
calldata charge for the changed capacity word; preallocated automatic output copying does
not add an EVM copy-word charge. Equal quotes therefore do not imply equal actual work.
Nevertheless, the existing length/memory/frame pricing covers the measured total and its
repetition growth. This checkpoint adds no tariff or coefficients.

The literal actual/node maximum is retained: the empty-return two-call control has
158,430,766 actual PGU / 6,304,617 node PGU = 25.129324430. Its complete quote is 161,246,448;
proposal startup is charged separately. Do not replace the literal denominator with the
complete quote when ranking, or interpret startup dominance as a scalable operation deficit.

The opposite tail also remains visible: the constructed 256-call caught-REVERT zero-copy
case has complete quote / actual = 8.171740982. This is a synthetic overquote, not a historical
Mainnet transaction or a new claim about Mainnet's roughly 2.085x aggregate result. No pricing
change means this checkpoint does not alter that earlier same-input historical comparison.

Five duplicate-input groups produced exactly equal PGU across duplicate capacity modes.
Differences between distinct tiny inputs still include the fixture/input data floor and must
not be used alone to fit a marginal coefficient. All 336 owned shared-memory objects were
verified cleaned up; resource guards remained active throughout sampling.

These observations support keeping the present pricing for this finite surface. They do not
prove that every composition is covered. Further tests should combine returned data with
frame depth, storage changes, rollback and exceptional child exits. Multi-user-transaction
construction and cases excluded by the legacy admission cap remain separate tool extensions.

## Measurement identity and reproduction

Native construction, quote and SP1 execution use frozen executable hashes, canonical input
hashes and matching public outputs. The quote model is
`537a965e51dd0416e3472a31cca8b067a85723fe4bbd08c52027ef3ca73f0e3b`
from alethia-reth `0f30f488c88635e38932843e67af2d918319f7e7`. The standalone sampler pins
SP1 SDK/executor 6.3.0 and explicitly selects ELF
`416f5a475b17aa15914fb9387159d9eae9ecd474e7104da65ac22512ab56da2e`.
These are execute-only PGU observations, not proof-generation or throughput measurements.
Parity with a different production SDK or the original calibration ELF is not claimed.

See [the bench usage guide](../../bin/zkgas/bench/README.md) for generation, screening,
measurement, resource limits, verified retention and report commands. Use a separate fresh
output root for each profile. Keep native rejects, failures and controls in the report.

The initial base catalog predates the optional profile selector. The final committed
generator reproduces all 90 request files, runtime, compiler record and Solidity source
byte for byte; only catalog generator identity and profile metadata differ. Both catalog
identities are retained in [Atlas #10](https://github.com/taikoxyz/raiko2-atlas/pull/10).
The [immutable fixture Release](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-return-copy-boundaries-v1)
contains the inputs and original observations; its actual remote download passed all archive
and metadata checks for 1,807 files.

The campaign also found and fixed a CLI resume bug: a previously rejected input stopped
`--continue-after-failure` before later screened cases could be measured. Continuation now
validates and reports the retained failure, creates no new attempt for it and proceeds while
retaining a nonzero aggregate exit. Corrupt/partial evidence and source/contract mismatches
remain fatal. All 18 original rejection records and their attempt counts are unchanged;
the final 15 IDENTITY measurements were then completed. The fixture runtime, input bytes,
model, ELF and executable identities did not change during this repair.
