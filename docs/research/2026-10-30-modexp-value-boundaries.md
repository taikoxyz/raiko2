# MODEXP value-boundary experiment

The objective remains actual SP1 PGU / new node-known zkGas. This cohort measures
input-value-dependent work invisible to the MODEXP route's native-gas-only price.
Source inspection alone does not establish a pricing deficit.

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
