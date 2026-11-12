# Frozen-model capacity validation

This checkpoint tests whether the SHA256 and ECMUL precision adjustments admit newly
oversized workloads. Prices remain frozen at
`80160649d7d29a85879cd9524b35c271a4b8638f64de94eef91d016e56121cf8`.
It supplements the [ECMUL calibration report](2026-11-11-ecmul-incremental.md) and
preserves the [original capacity checkpoint](2026-11-10-cap-frontier.md).

## Population and measurement boundary

The union contains 172 distinct measured canonical inputs: the original 85 capacity
inputs and the 87 current-program precompile, payload and ECMUL observations.
There are 160 eligible single-block proposals within 30M charged EVM gas, 12
multi-block proposals whose actual workload remains unallocated, and four additional
native-only controls. No source aliases collide. Existing SP1 actuals are reused;
only missing current-model quotes and the historical packing panel are replayed.

Actual means execute-only SP1 PGU from research ELF
`b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`,
SDK/executor/runner 6.3.0: `floor(raw ExecutionReport.gas * 10 / 191)`.
The 163 current-observer and nine earlier-observer reports remain separately pinned.
The earlier observer has retained finite same-input parity evidence; that qualification
does not certify another SDK, production ELF, GPU runtime or arbitrary observer build.

The mining estimate is the literal sum of the individually rounded block-known
quotes. The full proposal quote is separate. Raw Unzen difficulty retains its own
score unit and is never used as PGU. Multi-block actual PGU is never divided among
blocks or transactions.

## Offline admission frontier

The 3.6B node reference is an illustrative **single-block** sensitivity threshold,
not an enabled limit or a claim about a complete proposal's proving bandwidth.
The Unzen condition is its separate 100M raw score per block. These are maximum
observed workloads in a selected finite panel, not worst-case bounds.

| Condition | Eligible inputs passing | Maximum observed actual passing |
| --- | ---: | ---: |
| Raw Unzen score <=100M | 149/160 | 5,952,586,609 PGU |
| New node quote <=3.6B PGU | 149/160 | 2,246,391,418 PGU |

The identical counts hide different accepted inputs: 141 pass both, eight pass only
the new condition and eight pass only Unzen. No current-node-admitted input has
actual workload above 3.6B PGU in this panel. The measured high-workload cases
inside the old cap remain above the new reference after repricing.

Comparisons between model revisions use the same inputs on both sides:

| Matched comparison | Single-block denominator | Before passing | Current passing | Newly admitted actual >3.6B |
| --- | ---: | ---: | ---: | ---: |
| Original `c0877d65…` -> current | 73 | 61 | 65 | 0 |
| Parent `e8e574db…` -> current | 160 | 146 | 149 | 0 |

The four inputs newly admitted relative to the original checkpoint are SHA256
bulk cases already separately measured near the old cap. Relative to the immediate
parent, the three newly admitted inputs are the two caught off-curve ECMUL cases
with 200000 gas allowance and one/eight calls, plus eight repetitions of the
14-zero-scalars/one-dense-scalar mixed sequence. No previously admitted input is
removed in either matched comparison.

## Residuals and availability

Literal node underquotes remain visible: 118/160 eligible single-block inputs,
maximum deficit 153,438,578 PGU. Full-proposal deficits are 0/160 in this single-block
panel. Startup and control deficits are not subtracted from these statistics.
The twelve ineligible multi-block observations retain three full-proposal deficits,
respectively 11,862,881, 816,094,287 and 1,261,582,685 PGU. The zero single-block
count must not be reported as zero across all proposals.

The historical packing panel is the same 50 Mainnet inputs from October 27–29,
containing 9,600 existing blocks. Older-program actuals are excluded. All 9,600
block quotes fit the illustrative 3.6B node reference, as they did at both prior
checkpoints. The maximum current block quote is 1,590,862,383 PGU, down from
2,067,742,519 at the immediate parent and 2,266,932,279 originally.
Sixteen block quotes in thirteen proposals change relative to the immediate parent.

This checks existing block packing only. It does not simulate transaction repacking,
measure individual transaction rejection rates, establish broad application
compatibility, or bound a multi-block proposal by a single-block limit. Conservative
constructed KZG/BLS quotes and the accepted smaller residuals remain follow-ups.

The availability tradeoff is concrete: three of the new 87 inputs exceed the
illustrative node threshold despite actual workload below 0.5B PGU. They use valid
constructed precompile vectors; no claim is made that such inputs lack applications.

| Constructed input | Current node PGU | Actual PGU |
| --- | ---: | ---: |
| BLS pairing, two pairs, eight calls | 25,360,088,194 | 344,850,094 |
| KZG, one call | 3,631,603,749 | 192,670,356 |
| KZG, eight calls | 28,999,230,285 | 406,546,722 |

All historical blocks passing the reference does not resolve these route-level
overquotes. Future limit selection and application compatibility must preserve
this distinction; the current experiment changes neither their tariff nor a limit.

## Retained checks and reproduction interface

The read-only report tool remains `bin/zkgas/bench/cap_frontier_report.py`:

```sh
"$PYTHON_BIN" bin/zkgas/bench/cap_frontier_report.py \
  --manifest "$MANIFEST" --output "$REPORT"
```

Each manifest binds canonical input, native receipt, genuine current quote, actual
report, program and observer identities. Run the separately pinned observer-group
manifests individually; the union must preserve the qualification evidence and
deduplicate canonical input identities before reporting a common frontier.
The recorded evidence includes `MANIFEST-current.json`, `MANIFEST-parity.json`,
`OBSERVER-GROUPS.json`, `TABLE.json`, `CHECKPOINT-COMPARISONS.json`,
`HISTORICAL-COMPARISONS.json`, `SUMMARY.json` and source/resource receipts.

This checkpoint required 59 quote processes for 108 inputs: 58 missing measured-input
quotes and one 50-input historical batch. It used zero new SP1 invocations, zero
native-constructor invocations and zero retries. A historical postvalidator field-name
error was corrected against the already successful raw quote bytes; the original
attempt and correction receipt remain retained. Neither raw output nor its successful
quote invocation was repeated.

Independent review and numerical verification are recorded with the experiment
evidence. No tariff, runtime mode, SDK, guest ELF, admission policy or deployment is
changed by this experiment. The later EVM-gas-only saturation experiment remains deferred.
