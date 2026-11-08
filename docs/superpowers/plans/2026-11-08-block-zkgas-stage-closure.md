# Block-aware zkGas stage closure plan

> **For agentic workers:** Execute the checklist below in the existing worktrees. Use `superpowers:executing-plans` for execution; obtain independent review for substantive code changes. Do not start another open-ended experiment campaign.

**Goal:** Reach a documented, finite decision about the current block-mining-aware meter, with accepted residuals and a retained regression set.

**Architecture:** Keep ordinary GuestInput, the existing constructor/quote/measurement tools, and data-only Atlas fixtures. Classify costs by mechanism and observation surface, reuse retained evidence, and spend new SP1 executions only on named unresolved questions.

**Tech stack:** Existing Raiko/Reth Rust tools, managed Python environment, frozen SP1 execute-only backend, Markdown and existing producer-format manifests.

**Spec:** The user-approved direction is a finite block-level evaluation, prioritizing consequential underpricing while tolerating contrived overpricing and small documented residuals. [The evaluation standard](../../zkgas-evaluation-standard.md) supplies metric definitions; its wider draft promotion requirements are not adopted wholesale by this plan.

## Scope, baseline and alternatives

The selected approach is a mechanism inventory followed by a bounded closing set. Further exhaustive parameter/composition searches have no useful stopping point. Ending immediately would leave the existing evidence insufficiently organized to explain what was actually checked.

Freeze the following research baseline; later commits must not relabel measured binaries:

- Reth source checkpoint `a8e00f00fa0e6ab15e2936b19949927df34bcd5a` and Raiko tools checkpoint `16205d2ef44fbb050c716243c5d1cab0527f9e67`.
- Model `eea6f1da87ee129d2d9f759ae6cef01cb8ac93988376a86efb290398e6db5da6`, including the existing `J+2X` storage reserve and 384-PGU unit tariff.
- Experimental ELF SHA256 `b76baf2e0f3fa3fe6f8a20fe53dbf82f58601ed14b370dfc0f8d31db2069e034`, SDK6.3 execute-only. This is not production-SDK6.8 or completed-proof certification.
- Preserve quote401a's actual dirty-build provenance and historical native1368's constructor/state/gas-only qualification. The source checkpoints above do not change either artifact's identity.

At the start of this replan, the independently reconstructed **storage/account subset** contains 106 unique inputs, 9,682 blocks and 53 numeric actuals: 46 node-covered, 50 full-covered, seven positive node residuals and three positive full residuals. Maximum node residual is 1,597,968,115 PGU. Fifty historical proposals/9,600 blocks have no current-program actual; three controls are unmeasured. These are not whole-project coverage counts. The caught-transient pair is retained in [Atlas #30](https://github.com/taikoxyz/raiko2-atlas/pull/30).

## Global constraints

- Preserve three quantities: each integer `Q_node,b`, literal `Q_node = sum(Q_node,b)`, and full `Q_proposal`; compare both totals with the same whole-input actual `A`.
- Report `A/Q_node`, `A/Q_proposal`, signed gaps and charged EVM gas. Never substitute aggregate rounding, remove startup, invent per-block actuals, or interpret raw Unzen difficulty as PGU.
- The high-priority screen remains **strictly more than 2B missing node PGU within the matched 30M charged-gas domain**. It is a triage rule, not a deployment limit, precision certificate or waiver for zero-priced growing work.
- New controlled points in this stage retain whole-proposal charged gas at most30M, including any genuine prefix/Anchors. Every included block must satisfy its own gas rules. This conservative test domain is not a certificate for every possible30M-body-block proposal.
- Use actual gas budgets when considering mixtures; do not add independent maxima or repeat fixed startup. Lower-priority findings can interact, so their separate classifications do not establish a combined bound.
- Honest, compact, valid input is the scope. Deliberately bloated witness defenses, pre-Unzen compatibility, custom/preseeded backends, future forks, RISC0/Zisk, proposal shared budgets and allocating proposal overhead back to mining remain separate work.
- No guest/SDK/ELF/observer build, new public schema, version selector, runner, timer, deployment or admission activation. Keep Reth#260/Raiko#284/#296 Draft/do-not-merge.

## Fixed work and resource budget

There are **four tasks**, not an expanding queue of rounds:

| Task | Deliverable | Maximum new work |
| --- | --- | --- |
| 1 | One source/evidence inventory and frozen selection | Source/file review; zero SP1 executions |
| 2 | Retained regression and historical compatibility scorecard | At most32 controlled inputs and the fixed50 historical inputs; one initial pass and one reserved correction pass |
| 3 | Resolve only the selected evidence gaps | At most4 controlled and2 natural SP1 executions in total |
| 4 | Independent review and explicit stage decision | One consolidated report and one fixture publication batch if needed |

These caps are operational choices for this replan, not pricing parameters or a claim that six executions suffice for every finding. Unused slots stay unused. A failure consumes its slot and remains visible; do not silently replenish it with retries or replacements. If a material unresolved finding remains, report a named blocker rather than extending this stage automatically or declaring success.

The32/50 limits count fixed unique inputs, not unlimited retries per input. Each gets at most one native/quote attempt in the initial pass and one in a single reserved final-model pass if pricing changes: at most64 controlled and100 historical attempts, fewer when matching retained evidence is reused. Each of the at most six new Task3 inputs has one qualification native/quote attempt, plus at most one final-model quote if corrected. The SP1 cap is six total attempted invocations, including failures. There is at most one consolidated pricing-correction candidate in this stage; another material correction needs an explicitly bounded follow-up rather than restarting the cycle.

## Review focus

1. Model route presence may hide a collector/ownership omission; trace each mechanism from its real collection point to its price.
2. A full-proposal reserve may conceal a node deficit; keep surfaces separate, including MPT and startup.
3. A new layout may change shared state/population rather than only an opcode count; inspect lifetime and composition.
4. Reusing historical actuals across different ELF/SDK identities would produce false accuracy claims; mark them historical-only.
5. Synthetic overquotes do not establish normal usability; check retained natural inputs separately without inventing an admission limit.

## Task 1: Replace the round log with a mechanism inventory

**Files:** Update [the coverage inventory](../../research/zkgas-workload-coverage.md); retain its historical observations unchanged. Use existing model/collector sources and the linked research records.

**Produces:** A fixed table with collection site, growth variable, node/prove ownership, pricing expression, evidence/input identities, qualification limits and disposition for every row below. Record all enabled operation routes and intentional fallback policies from the frozen runtime specification; an arbitrary route count is not an opcode-coverage proof.

| Mechanism | Existing evidence to reuse | Question to close by review before selecting a run |
| --- | --- | --- |
| Dispatch, arithmetic and stack | Arithmetic/division/modular/EXP and normal-dispatch records | Any reachable growth dimension or execution/error route silently omitted? |
| Precompiles | ECMUL scalar mixture, MODEXP, BLS MSM, pairing and SHA records | Which value/length/branch cases have direct evidence versus deliberately conservative fallback? |
| Memory, copies and return data | Memory-word, MCOPY, KECCAK, return/copy and same-route analysis | Any unaccounted growth or consequential aggregation interaction, rather than another alignment variant? |
| Signed payload and transaction preparation | Existing payload analysis, authorization and access-list fixtures | Are accepted/attempted/rejected scopes and raw signed-byte ownership consistent? Current-ELF pure-payload refresh is optional unless this reveals a blocker. |
| Account journal and block cache | Account-density, cold/warm-first CacheState fixtures | Are repeated queries, distinct keys and cross-transaction population charged in the applicable scope? |
| Persistent/transient journal | Prefix, deletion, dirty-write, warm-tail/density and caught-transient fixtures | Are the seven retained node residuals understood as finite observations; is any new independent nonlinear mechanism still unaccounted? |
| Block finalization and trie work | Genuine-prefix transition/read-cache records and host completion sources | Which work is actually reserved by node, and which is exact prove-only information? No transfer of prove-only coverage into a node claim. |
| Calls, code lifecycle and receipts | Code/initcode/loading, CALL/DELEGATECALL, LOG and rollback records | Are attempted work, Unzen lifecycle behavior and terminal/result work retained? |
| Proposal-only work | Startup, parent MPT, blob/source and payload ownership | Explain the remaining `Q_proposal-Q_node`; do not redesign proposal budgeting in this stage. |

- [ ] Give each row one disposition: **evidence sufficient for this stage**, **accepted TODO with rationale**, or **named blocker**. A TODO rationale must discuss gas-bounded impact or why the path is outside the stated scope; “not measured” alone is insufficient. A reachable omitted growth mechanism cannot be downgraded merely because no >2B actual has been measured: it needs a supported severity argument or reserve, or remains a blocker.
- [ ] Freeze at most32 representative retained controlled inputs, covering all rows with applicable evidence and all seven known positive node residuals. Prefer existing current-EEA quotes; include controls. A row lacking compatible current-program evidence stays explicitly qualified.
- [ ] Freeze the exact50 historical inputs already used for sensitivity. Preserve all of them for compatibility reporting; they are not50 current-ELF PGU observations.
- [ ] Rank blockers by omitted growth, reachable gas-bounded impact and implementation cost. Freeze no more than two controlled hypotheses for Task3. Do not create another hypothesis just to use a slot.

**Done when:** Every row has a concrete disposition and evidence link; every selected input has an identity and inclusion rule. Any additional mechanism found during source review is classified explicitly in this one inventory before the selection closes.

## Task 2: Assemble the retained scorecard

**Interfaces:** Existing GuestInput, quote JSONL, observation/compare/report contracts and archived native results. No new execution schema or benchmark framework.

- [ ] Reuse matching current-model quotes; otherwise quote only the selected, compatible controlled inputs with the existing tool. Verify public output and input identities. Preserve failures/incompatible rows without replacing them or combining their numeric actuals across programs.
- [ ] For retained actuals compatible with the frozen program/backend, report both ratios, signed absolute gaps, charged gas, model/program identities and measured outcomes. Keep other rows historical-only. State the eligible denominator; do not use32 as a fictional measured count.
- [ ] Check all frozen50 historical inputs using existing evidence first. **All50 must have successful replay/state/public-output evidence and a valid current-candidate quote bound to the same input**; an unexplained new failure or missing/incompatible replay/quote is a closure blocker, not an excluded row. Use the bounded passes above for missing evidence. Historical actuals from another program do not waive this compatibility floor and do not become current-program PGU observations.
- [ ] Evaluate observed ranking and normal overpricing only where units/programs permit. An unspecified limit cannot produce a false-rejection rate. Keep Unzen's score separate; no numeric Unzen-score/PGU underpricing claim.

**Done when:** The selected regression has an explicit disposition for every row and all50 historical inputs meet the compatibility floor. Every price change reconciles to the intended changed component; an unexplained change is a blocker. Missing production-accuracy evidence limits the resulting claim; it does not silently turn this research stage into a backend migration.

## Task 3: Spend only the capped new-measurement budget

**Controlled slots: at most two pairs/four executions.** Each pair answers one Task1 blocker with a fixed control and a fixed stress input. Select shapes from source/retained evidence before opening their new actuals; bind geometry, input/code identities, gas budget and resource limits. If distribution-dependent construction is necessary, declare its finite reference-informed rule in advance, within the same pair budget. Do not adapt an already-closed panel or add a third point.

**Natural slots: at most two executions.** Use the lowest and highest recorded proposal IDs in the frozen historical set, deduplicating if identical. **At least one must yield a valid actual under the frozen research ELF/SDK**, bound to the identical current quote/input/public output. Otherwise close this stage as blocked on missing research-program evidence; a production-parity caveat cannot waive this requirement. This is a deterministic smoke comparison, not a representative accuracy certificate. Missing/incompatible inputs remain missing; do not silently substitute another proposal or rebuild the guest to fill the slots.

- [ ] Qualify any new generator once, with focused semantic/native tests and independent review proportionate to the change. Reuse existing qualification where the exact source applies.
- [ ] Before each actual, verify its legitimate execution, gas, input/public output, current quote and artifact/resource identities using the existing framework. Do not build a new metadata-gate stack for each point.
- [ ] Retain complete canonical outcomes and guarded cleanup. Add detailed hash/placement observation only if a named pricing decision requires it; generic process-hash reconstruction is not a default prerequisite to a whole-input price comparison.
- [ ] Classify positive node/full gaps separately. Prioritize a demonstrated >2B node shortfall in the matched gas domain or a real unpriced growth mechanism. Keep lower-priority residuals/TODOs under the existing complexity tradeoff.
- [ ] Make an explicit normal-workload decision for each successful natural observation: report both quote/actual ratios and explain the dominant overquote components. The user previously considered approximately2.5x tolerable short term; use a ratio above2.5x as a discussion trigger, not a deployment threshold or an automatic tariff target. A sampled natural ratio above that trigger, an unexplained historical regression, or a known normal-transaction exclusion remains a named blocker pending investigation or explicit user acceptance. The exception for contrived synthetic overquotes does not apply to natural inputs.
- [ ] If pricing changes, perform the required focused failure regression and independent review, then use the single reserved final-model pass on the fixed scorecard. Preserve original actuals and artifacts; do not repeat SP1 solely because the quote changed. Discuss material changes to earlier natural-data conclusions before proceeding further. Do not open another correction loop within this stage.

**Done when:** Every allocated slot has its recorded outcome and decision. Unsupported points and exhausted budgets are limitations/blockers, never inferred passing actuals.

## Task 4: Make and retain the stage decision

- [ ] Consolidate inventory, scorecard, known residuals and unresolved items into one closure report. Independently review the report/source delta once as a batch; do not require a separate engineering project per sample.
- [ ] Publish any new fixture data as one Atlas batch through its existing integrity checks. Keep tools and usage in Raiko. Record progress in `working.log`; no routine per-round confirmation is needed.
- [ ] Mark the outcome **closed with explicit limitations** only when there is no unresolved in-scope blocker, every inventory row has a justified disposition, the frozen regression has no unexplained new failure, and no known high-priority node deficit remains untreated. All50 historical inputs must meet the compatibility floor and at least one fixed natural input must have a valid current-research-program actual and an explicit normal-overpricing decision. Retained lower-priority residuals are allowed and stay visible. Neither missing research evidence nor a material natural-workload concern can be relabeled as only a missing production certificate.
- [ ] Otherwise mark **blocked on named findings**, listing the missing evidence or correction and its bounded follow-up. Reaching a sample/time cap alone is never successful closure.

This closes a **research stage for the block-aware model**, not global input coverage, the draft evaluation standard, production prover capacity, a numeric admission limit or deployment approval. Production SDK/program alignment is a separate bounded conformance check when that upgrade is pursued.

## After closure

Keep the fixed corpus as regression and resume ordinary shadow observation. Reopen model work for a new cost mechanism, a reproducible consequential deficit, a normal-transaction compatibility failure, or a deliberate fork/backend change. Extra variations of a covered synthetic mechanism and the previously requested EVM-gas-only maximum-workload experiment remain TODOs; they do not automatically reopen or prolong this stage.
