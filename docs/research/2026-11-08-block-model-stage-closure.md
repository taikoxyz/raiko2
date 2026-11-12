# Block-aware model: finite stage decision

Status: **closed with explicit limitations**. Consolidated independent review and remote data-integrity checks passed; [Atlas #31](https://github.com/taikoxyz/raiko2-atlas/pull/31) merged after57 successful checks at its exact reviewed head. This closes the [bounded research plan](../superpowers/plans/2026-11-08-block-zkgas-stage-closure.md), not production deployment or all-input certification.

## Scope and identities

The question is whether the current node-observable accounting has a known untreated consequential workload gap in the selected domain, with usable normal-input evidence. Keep the literal integer block-price sum and completed proposal price separate against whole-proposal actual PGU.

- Model: `eea6f1da87ee129d2d9f759ae6cef01cb8ac93988376a86efb290398e6db5da6`; no tariff, feature, discount or limit changed in this stage.
- Source checkpoints: Reth `a8e00f00fa0e6ab15e2936b19949927df34bcd5a`, Raiko tools `16205d2ef44fbb050c716243c5d1cab0527f9e67`.
- Research program: ELF SHA256 `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`, SDK/executor/runner6.3.0, execute-only `evm-gas-only` profile. No guest/SDK/ELF/observer rebuild.
- Quote401a keeps its actual Raiko200ccc-dirty/Reth5d-dirty identity. Existing historical constructors are construction/state/gas evidence, not current-EEA native-price implementations.

The frozen selection contains32 controlled inputs plus the original50 historical proposals. Natural observations were selected by lowest/highest proposal ID,44900/45309, before opening their new actuals. These are opened engineering and deterministic smoke inputs, not an untouched holdout or representative sample.

## Completed bounded work

| Task | Outcome |
| --- | --- |
| Mechanism inventory | Nine mechanisms classified; actual collection sites and node/prove ownership separated. Generated model check passes174 routes:150 ordinary opcode routes,6 confirmed wrappers,18 precompile routes. Four Amsterdam-only defined instructions remain outside Unzen/Osaka. |
| Retained controlled scorecard |32 successful current-model quotes with32 compatible retained actuals. Reused22 current quotes and executed10 missing quotations once each. No new controlled input or SP1 execution. |
| Historical compatibility | All50 inputs retain successful native state/root replay and current quotes, with input hashes/public outputs matched to prior evidence. No newly failing input. These50 rows were never interpreted as50 current-program PGU measurements. |
| New natural observations | Exactly2 SP1 invocations, both successful under the frozen program/backend, with matching public outputs and verified owned-resource closure. |

The32 retained raw actuals and both new actuals have the same ELF/SDK identities and semantically identical canonical executor options. Raw option JSON differs in serialization order of `retained_events_presets`, whose SDK type is a HashSet; sorting that set yields one semantic options identity. Original reports remain unchanged.

## Controlled results and residuals

Among the selected32 controlled inputs,9 node estimates and3 completed-proposal estimates are below actual. Maximum positive node shortfall remains **1,597,968,115PGU**; maximum completed-proposal shortfall is **1,261,582,685PGU**, both on the retained dirty-prefix24 near-budget input at28,263,673 charged gas. No new >2B/<=30M high-priority node point appears in this fixed set.

The seven known positive storage/account node residuals were deliberately retained. The other two node deficits are the MULMOD zero-repetition control (153,422,530PGU) and the selected full MCOPY layout (51,727,841PGU). They are not new independently identified high-priority mechanisms. Fixed startup remains in actuals and full quotes; no subtraction was used to make a node estimate pass.

The near-budget dirty-prefix24 residual is **underpriced**, not covered. It remains an accepted low-priority finding under the agreed complexity/severity rule. Other mixtures are not bounded by adding or multiplying these finite observations. Large constructed overquotes, including shared-prefix account/code cases, remain visible and are not used to justify raising a production limit.

The earlier106-input/53-actual storage/account checkpoint remains its original historical snapshot. The32-input selection is not an additional32 independent observations; the two new natural actuals qualify only their two historical inputs. Do not add overlapping cohorts or treat9,600 replayed historical blocks as9,600 independent current-program actuals.

## Same-program natural smoke comparison

All values below are whole-proposal PGU. Each proposal contains192 blocks. The node column is the literal sum of committed per-block integer prices; its deficit is not assigned to individual blocks.

| Proposal | Actual SP1 PGU | Literal node PGU | Full proposal PGU | Node / actual | Full / actual | Actual minus node |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
|44900|3,207,584,189|5,996,390,388|7,360,819,310|1.869441x|2.294817x|−2,788,806,199|
|45309|1,081,006,163|1,064,664,998|2,410,459,315|0.984883x|2.229829x|+16,341,165|

Both completed proposal quotes cover actual. Proposal45309's node total is slightly below actual, by16.341M PGU; this remains explicit. The30M controlled-block priority screen is not applied by pretending either192-block proposal has a measured per-block actual.

For44900, node's operation component is5.589B PGU, explaining most of its larger quote. For45309 it is0.676B. Completed pricing adds1.364B/1.346B above the respective literal node sums, from exact prove-side resource/MPT and once-only terms. These are price-accounting explanations, not isolated actual component measurements. Shared dispatch diagnostics are already inside operation totals and must not be summed twice.

Normal-workload decision: these two points support a limited smoke pass. Full overquotes2.295x/2.230x stay within the approximately2.5x short-term discussion range; no tariff refinement is justified by this closing sample. They do not establish a population quantile, universal transaction availability or a production capacity deadline. No admission limit has been selected, so no future-filter false-rejection rate is claimed. No new population-level Unzen superiority claim follows from two points; raw Unzen scores remain separate from PGU.

## Verification and operational limits

- Fresh source check: managed Python `crates/evm/src/zk_gas/pricing/generate.py --check` passes174 routes. Routing checks are distinct from independently measuring174 tariffs.
- Scorecard checks re-read canonical input and raw report hashes, same-program/backend identities, SDK normalization `floor(raw_gas*10/191)`, quote/public-output bindings, every node ceiling/literal sum and full total ceiling. All50 historical outputs match retained prior native-replay evidence.
- Independent review identified missing charged-gas columns in the initial derived scorecard. All32 retained native reports were then bound by input/public output and original hashes; the additive scorecard and fixture index now retain every charged block value and whole-proposal sum, including Anchor/prefix. All32 totals are at most30M; maximum29,999,575gas. No native, quote or SP1 execution was repeated to add these fields.
- New44900 raw SDK gas is61,264,858,019;45309 is20,647,217,723. Their normalized PGU and public outputs match the table. Frozen code/artifact identities are unchanged after both executions.
- Existing process guard limits are6GiB PSS,1200seconds,192MiB output,16MiB log and576MiB disk floor. Peak sampled PSS is4,576,874,496/4,575,053,824bytes; wall time91.340/28.128seconds. Both receipts' four exact owned SDK objects were already absent; no unrelated resource was inspected or removed. Output bounds are sampled, not a kernel file-size guarantee for SP1 mmap execution.
- A private scorecard reader initially expected SDK raw gas at the report root instead of `execution_report.gas`. It was corrected after one successful quotation; that exact quotation was reused. No quote, input construction or SP1 measurement was repeated.
- No product code changed, so no local Rust/guest rebuild or unrelated test lane was run. Atlas's existing CI separately builds its pinned host tools for fixture replay; no guest was rebuilt. Consolidated independent review, actual remote download and the unchanged Atlas integrity consumer passed. All57 PR checks passed at head839d0032; merged main1f9b744e has the identical reviewed tree. This does not predeclare the subsequent main-branch CI run green.

## Retained data

The [immutable fixture](https://github.com/taikoxyz/raiko2-atlas/tree/1f9b744ea34adbb18ad897beeaf413490e2b6ed6/fixtures/benchmarks/unzen-block-model-closure-v1) provides34 canonical GuestInputs/current quotes/raw actuals,32 native gas reports, standard quote input manifests and all50 historical current quotations. Its nonlatest [Release asset](https://github.com/taikoxyz/raiko2-atlas/releases/tag/fixture-unzen-block-model-closure-v1) is12,371,344bytes, SHA256 `a2dfc7c52128be4ea8ba150fee45d058fa5714f4b29797f9b51654c3b2618e82`, with175 regular members. Prior fixtures/releases are unchanged. Execution tools stay in Raiko; no runner, decoder or executable is added to Atlas.

## Accepted limitations and reopening rules

Keep the known smaller node/full residuals, more exact MPT topology, current-program pure-payload refresh, remaining route/branch mixtures, and additional ordinary traffic as documented follow-ups. MPT path shape is not proven harmless; accepted triage uses the existing unique-resource/transition reserves and retained trie-shape evidence. No global worst-case or whole-map hash-placement claim follows.

Future production ELF/SDK alignment, actual proof latency/throughput, admission-limit selection, proposal shared budgets, deliberately redundant witness defenses and other backends are separate bounded work. The current research program is frozen; a future fork/backend change deliberately reopens the relevant conformance question.

The selected workload evidence and normal-compatibility minimum now support ending this research stage **without repricing**. Resume fixed-corpus regression and ordinary shadow observation. Reopen for a new cost mechanism, reproducible consequential underpricing, normal-input compatibility failure, or deliberate fork/backend change; another variation of an already covered synthetic family does not automatically start a new campaign.
