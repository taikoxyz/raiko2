# MODEXP value-boundary experiment

The objective remains actual SP1 PGU / new node-known zkGas. This cohort measures
input-value-dependent work invisible to the MODEXP route's native-gas-only price.
Source inspection alone does not establish a pricing deficit.

We use **proving-cost underpricing** for this search. Underpricing describes a
resource price that fails to track actual work; [EIP-7667](https://eips.ethereum.org/EIPS/eip-7667)
uses this term for the mismatch between CPU-oriented gas and ZK proving costs.
Here the denominator is the new node quote, with complete quotes and same-program
increments retained as separate diagnostics.

## Frozen design

Reuse the canonical ProgramRequest constructor, explicit `evm-gas-only` profile,
model `537a965e51dd0416e3472a31cca8b067a85723fe4bbd08c52027ef3ca73f0e3b`, and
ELF `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`.
There is no guest or tariff change in this experiment.

The pinned guest uses Aurora MODEXP 1.2.0. Odd moduli select Montgomery arithmetic,
powers of two select wrapping exponentiation, and other even moduli select CRT,
including an inverse calculation sensitive to the power-of-two factor.

For byte width `L` in 128/256/512, use base `2^(8L)-1`, one exponent byte,
equal declared base/modulus widths, and exponent 0/1/3. Select modulus:

- odd: `2^(8L)-189`;
- power of two: `2^(8L-1)`;
- even CRT: `3 * 2^(8L-2)`.

These 27 single-call inputs have source-derived precompile gas 512/2048/8192 by
width. Add a zero-call CRT/exponent-3 control at each width, then CRT/exponent-3
width-512 cases with four and sixteen calls, giving 32 plans. Exponent zero is
an early-return control. The full-width base ensures exponents one and three
reach parity dispatch rather than a small-base optimization.

Equal precompile gas does not imply equal whole-node quotes: wrapper, memory,
signed payload and completion resources remain separately observed.

## Construction and validation

Add `ModexpValues.sol`, `modexp_values.py`, focused tests and usage text beside
the existing producers. Use one runtime, copy prepared input once, preallocate
separate output, clear every output word before each STATICCALL, and verify
success, exact output length and every word against Python modular exponentiation.
Exact native STATICCALL and root RETURN counts gate measurement. Validate payload
lengths, MODEXP headers, widths and loop counts before the first call.

Tests cover independent closed-form outputs, the exact 32-plan matrix, retained
request/compiler hashes, no-overwrite behavior, real native outputs/opcode maps,
and malformed payload/header, incorrect output and excessive-count rejection.
Require independent review and native verification before SP1 sampling.

Freeze a fresh corpus using the existing reviewed program and resource guards.
Retain controls, failures, literal rankings, full quotes and matched increments.
Change tariffs only if measured evidence warrants it. Tools stay in Raiko and
fixed data stays in Atlas. EVM-gas saturation remains deferred.

## Completed observations

All 32 planned inputs completed actual SP1 execution with the same program.
There are no failures, pending inputs or complete-quote deficits. All nine
single-call value variants at each width have identical observed opcode maps
and operation prices. This equality is checked directly in the quote; charged
transaction gas is not substituted for native precompile gas.

At width 512 and exponent three, odd/power-of-two/CRT moduli measure respectively
162,787,461 / 161,684,139 / 165,125,946 actual PGU. All three node quotes are
118,129,176 PGU and complete quotes are 273,071,263 PGU. The hidden algorithm
branch changes actual work, but this finite cohort does not show underpricing.

The width-512 CRT/exponent-three repetition ramp is:

| Calls | Actual proposal PGU | Node-known PGU | Complete quote PGU | Charged user EVM gas |
| --- | ---: | ---: | ---: | ---: |
| 0 | 161,260,947 | 6,587,016 | 161,528,847 | 68,870 |
| 1 | 165,125,946 | 118,129,176 | 273,071,263 | 68,900 |
| 4 | 176,696,613 | 452,719,170 | 607,661,257 | 88,111 |
| 16 | 222,983,212 | 1,791,079,147 | 1,946,021,233 | 229,363 |

All three repetition increments are covered. The largest actual/node increment
is 0.0346505662: the first call adds 3,864,999 actual PGU versus approximately
111,542,160 node-known PGU. The current conservative tariff covers this measured
surface without an additional value selector. No pricing coefficient changes
are supported by these observations.

The literal actual/node maximum remains the width-128 zero-call control:
161,095,735 / 6,290,258 = 25.610354138. Its numerator includes proposal startup,
which is absent from the node subtotal. Retain that result alongside the full
quotes and matched increments rather than relabelling it as precompile work.

The six focused tests, independent 25-case native/16-quote check, all 32 harness
verification joins and independent raw-data audit passed. The latter checks
normalization, canonical inputs/public outputs, program/profile/source identities,
literal rational rankings, all nine value pairs and repetition increments.
Peak process-tree PSS was 1,471,420,416 bytes; all 128 exact owned resources were
confirmed absent in the host namespace. These are finite execute-only observations,
not proofs, GPU throughput measurements, or coverage of every MODEXP value/length.

The immutable data-only cohort is in [Atlas #13](https://github.com/taikoxyz/raiko2-atlas/pull/13).
Both local integrity checks and the unchanged workflow against its published Release
verified all 554 archived files. The archive preserves raw observations, exact portable
projections and canonical inputs; the tools and program source remain in Raiko.
