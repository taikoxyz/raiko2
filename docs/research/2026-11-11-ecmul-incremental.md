# ECMUL incremental-tariff refinement

The finite follow-up completed 37 new execute-only SP1 observations and 192 fresh candidate quotations. The change removes one PC07 incremental coefficient, while retaining its complete accepted constant/native slope and scalar structural model. It reduces input-insensitive overquotes without a new selector or collector rule. The separate [50-observation precompile/payload panel](2026-11-11-current-program-coverage.md) required no repricing.

## Identities, selection and policy

- Research ELF `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`; SP1 SDK/executor/runner 6.3.0; explicit `evm-gas-only` policy. Actual PGU is `floor(ExecutionReport.gas * 10 / 191)`.
- Parent model `e8e574db50607857daadfd79f24e2cd6773177f6b46e4ee950f168379b4cab57`; candidate `80160649d7d29a85879cd9524b35c271a4b8638f64de94eef91d016e56121cf8`.
- Retained producer source `9a07d4e30b06f5651d37088d1ce497516c0be383`; candidate Reth implementation `31b60b2a6e0d09f66b48ccd204a13526f2fecc3c`. The actual candidate quote build predates these retention commits and records its real dirty source snapshot; its executable SHA256 is `066b970929c3ef3247c2bd68dd8679187da8c49fbcc970bb353fb6a6cc9511a7`.
- [Successful scalar/point producer](../../bin/zkgas/bench/cases/ecmul_tariff.py), [caught-failure producer](../../bin/zkgas/bench/cases/ecmul_failures.py), existing [custom scalar-mixture producer](../../bin/zkgas/bench/cases/ecmul_scalar_mix.py), focused tests and [usage](../../bin/zkgas/bench/README.md#finite-ecmul-scalar-and-point-probes) remain reusable. Generation does not invoke a prover or network.

The original fit has 18 inputs: points G/2G, scalars zero/two/dense and counts 0/1/8. Six intermediate-work observations cover sparse W255 and dense W254. Independent source review required two additional finite gates before removing the coefficient globally: three 14-zero/one-dense mixtures and six caught off-curve failures at native allowances 6000/200000. Each group retains its own 0/1/8 controls. These 33 opened development observations support the numerical policy. Four point-3G observations at zero/four calls were selected before any fit actual and opened only after the candidate identity was sealed; their outcomes were not used to refit it. All 37 actuals completed once without failure or retry.

For event j, `g_j` is consumed native gas and `W_j` is reduced-scalar bit length plus popcount. PGU pricing is:

```text
A_j = max(20,000,000, 131.390224983920708487 * g_j)
I_j = 29,901 * g_j                    # parent only
S_j = 572,936 * W_j
parent PC07 = max(sum A_j, sum I_j, sum S_j)
candidate PC07 = max(sum A_j, sum S_j)
```

The maximum remains after route-wide block accumulation. A successful 6000-native-gas call previously received at least 179.406M PGU even for a zero/tiny scalar. The candidate retains 20M accepted PGU and the full scalar price. Dense W507 work still costs 290.478552M per call, so its price is unchanged. A failed call can burn its full supplied allowance: at 200000 gas the retained accepted contribution is 26,278,044.9967841416974 PGU, with its fractional fixed-point value preserved until the final quote ceiling. It is not a constant-20M model for all failures.

No Unzen coefficient, admission limit, global discount, route shape, namespace, callback, memory/resource ownership or guest policy changes. Disabled/Shadow behavior remains non-enforcing.

## Selected whole-input results

All rows below contain one block and measure complete proposal actuals, including startup. Node is the literal block estimate; full is the complete proposal quote. Matched increments do not replace these quantities.

| Input | Actual PGU | Parent node PGU | Candidate node PGU | Candidate full PGU |
| --- | ---: | ---: | ---: | ---: |
| Point G, scalar zero, 8 calls | 161,184,418 | 1,443,175,248 | 167,927,248 | 322,869,590 |
| Point G, scalar two, 8 calls | 168,806,099 | 1,443,175,248 | 167,927,248 | 322,869,590 |
| Point 2G, scalar two, 8 calls | 169,026,564 | 1,443,175,248 | 167,927,248 | 322,869,590 |
| Point G, dense W507, 8 calls | 2,098,339,882 | 2,331,755,664 | 2,331,755,664 | 2,486,698,006 |
| Point 2G, dense W507, 8 calls | 2,098,723,388 | 2,331,755,664 | 2,331,755,664 | 2,486,698,006 |
| Intermediate W254, 8 calls | 1,131,043,558 | 1,443,192,702 | 1,172,150,654 | 1,327,092,997 |
| Intermediate W255, 8 calls | 1,145,206,963 | 1,443,192,702 | 1,176,734,142 | 1,331,676,485 |
| 14 zero + 1 dense, 15 calls | 403,758,093 | 2,699,703,814 | 308,613,814 | 463,556,157 |
| 14 zero + 1 dense, 120 calls | 2,101,911,525 | 21,541,091,853 | 2,412,371,853 | 2,567,314,196 |
| Failed calls, allowance 200000, 8 calls | 161,181,752 | 47,849,544,383 | 218,168,743 | 373,111,086 |
| Reserved point 3G, scalar two, 4 calls | 165,045,278 | 725,403,394 | 87,779,394 | 242,721,737 |
| Reserved point 3G, dense, 4 calls | 1,131,833,268 | 1,169,693,602 | 1,169,693,602 | 1,324,635,945 |

Across 37 inputs, full-proposal underquotes remain 0/37. Literal node underquotes increase from 17 to 27, including ten newly uncovered small inputs; the maximum positive node gap remains 153,407,027 PGU. The four reserved observations have three node deficits, maximum 153,406,095 PGU, and zero full deficits. These startup/control deficits remain findings, rather than being subtracted away to label node pricing exact. All 35 within-group count contrasts, including the two reserved contrasts, are covered by both node and full increments. They are conditional whole-input differences, not isolated precompile actual costs or a uniform extra safety margin.

The 14:1 mixed gate matters because `max(sum A, sum S)` is smaller than the sum of individual maxima. At one repetition, candidate route A is 300M and S is 290.478552M; the full quote still covers the measured 403.758093M workload. This finite result addresses the tested crossover, not every possible mixture or gas-saturation schedule.

## Fixed regression and Unzen comparison

Every candidate quote is a new native replay of its retained canonical input. The 192-row selection consists of the same 32 pressure inputs, 45 ordinary fit/reserved inputs, 76 already-opened Hoodi observations, two original later Hoodi observations and the 37 new ECMUL actuals. Requoting creates no new SP1 observation. Input/public output, Unzen score, selectors, startup/MPT/resource terms and every unrelated quote component remain equal; only PC07 operation contributions and dependent totals can decrease.

The pressure set retains nine literal node and three full-proposal deficits, maximum 1,597,968,115 and 1,261,582,685 PGU respectively. Neither count nor maximum increases. Ordinary-case quotes are numerically unchanged. No newly measured legal single-block input has a >2B PGU node deficit. This is a finite regression, not a proof of a global 30M-gas maximum. Multi-block actuals remain whole-proposal observations.

The original opened Hoodi split stays fixed at 18 training, 55 evaluation and three boundary observations. A single scale is learned from training sums separately for each score, then evaluated on the 55 evaluation rows. No per-row fitting or spam-derived tariff adjustment is performed. This scale-normalized workload-shape diagnostic permits comparison with Unzen's different unit; raw Unzen values are never treated as PGU or given literal PGU underquote labels.

| Evaluation metric | Parent e8 | Candidate 801 | Unzen | Constant baseline |
| --- | ---: | ---: | ---: | ---: |
| Node training-scaled WAPE (%) | 14.910261 | 2.793784 | 6.283565 | 6.063455 |
| Full training-scaled WAPE (%) | 8.465111 | 2.683576 | 6.283565 | 6.063455 |
| Node Spearman | 0.756421 | 0.959596 | 0.512799 | undefined |
| Full Spearman | 0.762266 | 0.965152 | 0.512799 | undefined |

Separately, literal node sum / actual sum changes from 1.201900947 to 1.074391996; full sum / actual sum changes from 2.230247360 to 2.102738408. Both candidate surfaces retain zero underquotes across these 55 whole-proposal observations. Scaling is not used for these literal protection quantities.

The controlled evidence identifies a concrete source of poor precision: an input-insensitive native-gas floor on cheap ECMUL calls. Removing that floor while retaining measured scalar work substantially improves both shape metrics in this opened cohort and preserves its literal coverage. The candidate now outperforms training-scaled Unzen on WAPE in this fixed research cohort. It does not establish a universal advantage across applications, a historical transaction admission policy or production-program/GPU proof-time accuracy. The remaining approximately 2.1x full quote ratio reflects other conservative completion components and is distinct from the much closer node subtotal.

## Verification and limits

Independent source review authenticated the immutable parent, 33 raw endpoint facts and exact one-coefficient/table delta. Independent native checks cover signatures, actual point/scalar/caught-failure guards, exact charged gas, loop/opcode counts, padding/truncation/reduction, output/status and mutation negatives. Final Reth behavior tests passed 88/88, including five new ECMUL boundary/aggregation regressions; 77 pricing-generator tests and both generation checks passed. Host-only candidate build completed in 44.496 seconds with 853,282,816-byte peak process-tree PSS. No guest, ELF or SDK observer was rebuilt.

The sampler stayed within the agreed 6GiB sampled PSS guard; ECMUL's maximum observed process-tree PSS was 4,548,798,464 bytes. Original-host read-only checks verified all 148 owned SDK object names from the 37 executions absent after cleanup. The research manifest is a declared execution policy, not an arbitrary-binary build attestation. Model-declared production ELF/VK identity and the measured b76 target remain distinct; fixture profiles expose both scopes explicitly.

The complete Reth `just clippy` check did not pass: the pre-existing unused `RequiredInput::Log` variant is rejected under warnings-as-errors. An independent pristine-parent check reproduces it with byte-identical model code and unchanged zero Log constructors. No warning was silenced. Research branches remain Draft/do-not-merge; no deployment or enforcement follows from these results.

Future work remains scoped: production target/fork conformance, normal-transaction availability at a chosen limit, accepted sub-2B residuals, additional application diversity and the deferred full-EVM-gas capacity experiment. Highly conservative KZG/BLS constructed quotes from the first panel remain explicit precision follow-ups; this ECMUL change does not silently adjust those routes.
