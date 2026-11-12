# Current-program precompile and payload coverage

Fifty new execute-only SP1 observations completed:36 precompile inputs and14 signed-payload inputs. All raw input/public-output/program/model joins and literal quote arithmetic passed independent verification. No pricing, limit, guest, SDK or deployment changed in these two panels.

## Scope and retained tooling

- Research ELF `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`; SDK/executor/runner6.3.0; explicit `evm-gas-only` policy.
- Model `e8e574db50607857daadfd79f24e2cd6773177f6b46e4ee950f168379b4cab57`; quote executable SHA256 `09e5762b84ca1c7f38261e70193e317e0005f2254056463a20554cda3662f7a7`.
- Producer source commit `f15a379cc8d5efd6022c8cf7b5550acc483b0a0e`. The pre-existing native constructor and sampler retain their actual executable identities; this commit is not their claimed build provenance.
- Reusable [precompile producer](../../bin/zkgas/bench/cases/precompile_boundaries.py), [vector bank](../../bin/zkgas/bench/cases/precompile_boundary_vectors.json), [payload producer](../../bin/zkgas/bench/cases/signed_payload.py), focused tests and [usage](../../bin/zkgas/bench/README.md#remaining-precompile-boundaries). No compiler or crypto package is needed by generation.
- Fixtures retain canonical GuestInput, actual raw reports, native/quote observations, source/request hashes and outcomes. Source/program/model are distinct identities.

PGU is exactly `floor(ExecutionReport.gas*10/191)`. Node is the literal sum of per-block ceilings; full is the complete proposal quote. All new inputs contain one block, but actuals remain whole one-block-proposal measurements including startup. Raw Unzen difficulty remains a different score unit.

## Remaining twelve precompile addresses

The selection was fixed before actuals: one valid nontrivial vector per address at0/1/8calls,36 distinct canonical inputs. RIPEMD uses4096 input bytes, BLAKE2F4096rounds, G2MSM a dense254-bit scalar, and BLS pairing two nonzero pairs. Other inputs include valid nonzero signatures, non-infinity curve addition, a nontrivial KZG opening and official EIP2537 field-map vectors. A successful false pairing returning zero is distinct from a failed call.

| Eight-call vector | Charged whole-block gas | Actual PGU | Node PGU | Full PGU |
| --- | ---: | ---: | ---: | ---: |
| ecrecover-valid | 216,131 | 161,563,083 | 167,910,164 | 322,852,250 |
| ripemd160-4096 | 384,256 | 162,841,747 | 52,029,744 | 206,971,831 |
| bn254-add-double | 192,514 | 164,755,710 | 167,935,256 | 322,877,342 |
| blake2f-4096 | 225,232 | 167,222,902 | 466,914,731 | 621,856,818 |
| kzg-valid | 593,754 | 406,546,722 | 28,999,230,285 | 29,154,172,628 |
| bls-g1-add-double | 197,680 | 161,897,196 | 58,897,182 | 213,839,269 |
| bls-g2-add-double | 204,995 | 162,321,354 | 101,335,582 | 256,277,669 |
| bls-g2-msm-dense | 376,418 | 244,612,331 | 1,086,441,437 | 1,241,383,780 |
| bls-pairing-k2 | 1,023,736 | 344,850,094 | 25,360,088,194 | 25,515,030,280 |
| bls-map-fp | 236,172 | 171,664,908 | 921,245,049 | 1,076,187,136 |
| bls-map-fp2 | 384,600 | 192,807,419 | 3,349,609,289 | 3,504,551,376 |
| p256-valid | 248,185 | 162,087,114 | 767,092,173 | 922,034,260 |

All36 actuals completed once without failure/retry. Literal node underquotes remain24/36, maximum153,403,163PGU. All36 full quotes cover actual. All24 own-zero-to-one/eight conditional workload increments are covered by both node and full increments; subtraction does not erase the absolute node gaps.

Some route prices are highly conservative: KZG and BLS pairing are prominent here. These are constructed inputs, not a measurement of historical transaction rejection at a selected new limit. No price was lowered from this panel. Eight calls and a maximum1,023,736 charged block gas do not establish gas saturation or a global30M-gas bound.

Source review corrected a P256 vector provenance hash and strengthened fresh-output guards: buffers now start with the full complement of expected output, so missing all-zero pairing copies cannot pass. Exact data/status/returndata/address0x100/counter/gas guards passed independent literal bytecode and native checks. Missing-copy, wrong-output, truncated-address and insufficient-gas mutants are rejected.

## Pure signed-payload refresh

The fixed14-input panel uses a one-byte STOP runtime, one/eight users, fixed3M declared gas per transaction, lengths0/32/4096/65536 and zero/sequence contents. Empty data has one canonical control per count. Sequence byte i is `(73*i+19)%256`. There is no user storage, precompile, LOG or deployment work.

| Payload | Users | Signed envelope bytes including Anchor | Nonempty inputs including Anchor | Charged block gas | Actual PGU | Node PGU | Full PGU |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| zero/0 | 1 | 318 | 1 | 188,355 | 160,981,939 | 7,544,808 | 162,486,639 |
| sequence/32 | 1 | 350 | 2 | 189,635 | 161,000,658 | 7,569,384 | 162,511,215 |
| zero/32 | 1 | 350 | 2 | 188,675 | 160,997,489 | 7,569,384 | 162,511,215 |
| sequence/4096 | 1 | 4,417 | 2 | 351,715 | 161,555,289 | 8,610,536 | 163,552,367 |
| zero/4096 | 1 | 4,417 | 2 | 229,315 | 161,547,361 | 8,610,536 | 163,552,367 |
| sequence/65536 | 1 | 65,859 | 2 | 2,802,115 | 170,094,296 | 24,339,688 | 179,281,519 |
| zero/65536 | 1 | 65,859 | 2 | 843,715 | 170,083,541 | 24,339,688 | 179,281,519 |
| zero/0 | 8 | 1,081 | 1 | 335,355 | 162,181,551 | 8,963,811 | 163,905,642 |
| sequence/32 | 8 | 1,337 | 9 | 345,595 | 162,301,658 | 9,160,419 | 164,102,250 |
| zero/32 | 8 | 1,337 | 9 | 337,915 | 162,298,939 | 9,160,419 | 164,102,250 |
| sequence/4096 | 8 | 33,873 | 9 | 1,642,235 | 166,731,087 | 17,489,635 | 172,431,466 |
| zero/4096 | 8 | 33,872 | 9 | 663,035 | 166,732,727 | 17,489,379 | 172,431,210 |
| sequence/65536 | 8 | 525,409 | 9 | 21,245,435 | 234,095,687 | 143,322,851 | 298,264,682 |
| zero/65536 | 8 | 525,409 | 9 | 5,578,235 | 234,127,936 | 143,322,851 | 298,264,682 |

All14 actuals completed once without failure/retry. Literal node underquotes remain14/14, maximum153,437,131PGU. All full quotes cover actual. All12 same-count-empty byte increments and all7 same-payload one-to-eight count increments are covered by both node and full increments. No new >2B-PGU gap appears; this finite current-program refresh does not select a tariff change.

Independent decoding reconstructed signed EIP1559 RLP, authenticated transaction hashes/signers and checked exact calldata, nonces,3M envelopes, receipts, STOP counts and runtime code. Canonical transaction bytes exactly join the quote selector and include signatures/envelope fields/Anchor. The nonempty-payload transaction selector counts nonempty input/calldata, not every nonempty serialized envelope: the empty one-user control is318 signed bytes but only one nonempty input (Anchor).

Zero and sequence variants can differ in signature-encoding length even at equal physical calldata size; their actual canonical byte counts are retained. EIP7623 changes charged EVM gas sharply between these patterns while measured proving work changes little. EVM gas is not used as a substitute for the measured workload target.

## Verification and bounded conclusion

- Independent source review and independent behavioral/raw-data verification passed. Native selectors/counts/gas, canonical inputs, raw normalization, public outputs, model/program/options, all node component ceilings and full composition were rechecked.
- Native/bytecode qualification includes valid nontrivial vector oracles and injected failure negatives. Quoting does not execute SP1; actuals are independently measured.
- Shared existing sampler lock serialized work with the continuous Hoodi collector; no service or collector pin changed.
- Guard:6GiB sampled descendant PSS,1200seconds/stage, bounded logs/output and8GiB minimum free disk. Precompile peak2,140,789,760bytes and summed actual execution209.565seconds; payload peak1,431,716,864bytes and75.350seconds. These are CPU estimator measurements, not GPU proof throughput.
- Read-only checks in the original host mount namespace verified36+14 exact four-object SDK receipts,200 owned names absent after execution. No unrelated object was removed.

This closes the bounded current-program verification of the selected twelve precompile vectors and the isolated payload refresh. Unmeasured operands, invalid/early-return branches, MSM/pair batch shapes, full-gas packing and production ELF/SDK parity remain separate boundaries. Known absolute node deficits stay visible. No high-priority deficit was discovered, and no global containment or production-admission readiness claim follows.

Next approved precision work is cheap ECMUL floor applicability, with multiple valid points/scalar classes, frozen fit/reserved selections and preservation of heavy/mixed regressions. It does not expand the completed two-panel measurement population.
