# Normal opcode dispatch reserve implementation plan

> **For agentic workers:** Use subagent-driven-development, independent adversarial review and behavioral verification. This is a candidate development refinement; no merge, deployment or activation is authorized.

**Goal:** Test an additive normal-opcode reserve against the repeatable deficits in 60 independently qualified and measured development inputs, preserving both proposal and node comparisons.

**Architecture:** Preserve every existing coefficient, floor and discount in immutable parent model bbce9be0. Add an independently counted, checked, additive reserve once per normally priced dispatched Opcode/ConfirmedSpawn event. Apply it after route candidate maxima so it cannot be swallowed by an existing floor. Precompile-body events, feature-error pricing and selected-but-undispatched diagnostics retain their existing behavior.

**Tech Stack:** Existing Reth Rust pricing/collector, authenticated Python successor generation, unchanged Raiko native quote and SP1 estimator composition.

**Spec:** The session's authorized SP1 pricing campaign, the arithmetic-density research record, and the constraints below.

## Frozen candidate policy and evidence

- Opened development evidence: 24 arithmetic, 12 paired DUP/POP controls, 24 balanced stack inputs; 60 matched increments, all same-pattern/operand N0 or N4096 references.
- Candidate extra PGU/event is `ceil_to_8(max(0, max((5/4 * actual_delta - parent_node_scaled_delta / scale) / extra_completed_user_opcodes)))`. Exact rational replay gives maximum about58.3843496621 and candidate64 PGU/event.
- Independent panel SHA256 `7c6f4ce7cfa98ddbcabbaeabdccb30b7088666ef22c96b80e3b946e68b65a7c3`; independent residual audit SHA256 `69d3a6ab96c756035a4f0d369b2993894254122481f994bdda6b4a03b2560e72`.
- These event deltas are from completed user-code opcodes and exclude Anchor. Actual candidate replay must confirm that the full normal-dispatch count differences equal the qualified deltas. Failure to confirm this invalidates the proposed fit interpretation.
- This is an empirical composite reserve, not a measurement of isolated dispatch overhead or proof that original calibration omitted Taiko metering. No prospective observation is used to select64.
- Preserve literal actual/node, actual/proposal, both independent underpricing flags and all controls. The proposal-only startup/observability gap is not allocated back to mining by this change.

## Implementation ownership and checks

1. **Reth candidate.** Implementation owner changes only maintained Reth shadow-pricing sources, generator/evidence/tests and pricing docs. Add an explicit normal dispatch count to authoritative route accumulation and any directly required snapshots. Count only accepted normal Opcode/ConfirmedSpawn events, once; feature errors, Precompile and diagnostic events do not increment it. Retain committed/attempted/discard/rollback semantics and checked arithmetic. Direct inputs must state the new selector explicitly; do not silently fabricate missing observations.
2. **Price composition.** Expose the reserve contribution as an auditable subterm of operations pricing. `operations_scaled` includes it exactly once; consumers must not add the exposed subterm again. Both direct per-event pricing and block aggregate pricing must agree after existing max/floor composition, with no per-event ceiling.
3. **Authenticated generation.** A new successor exporter binds immutable bbce parent, the 60 raw-derived observations/count deltas, fixed 25% increment margin, rounding quantum8 and coefficient64. Preserve inherited provenance as inherited, not reexecuted calibration. Regenerate generated Rust/artifacts through the exporter and existing generator; no hand-edited generated coefficients.
4. **Regression and review.** Cover plain/inspected parity, normal contextual and confirmed-spawn routes, selected-but-undispatched zero charge, Precompile/feature-error exclusion, route max dominance, merging/count overflow with atomic failures, included reverts/halts, committed versus attempted/discarded prefixes and direct-input requirements. Run focused Rust and Python pricing suites; independent reviewer/tester investigate real findings. Preserve default Unzen behavior.
5. **Host quote integration.** Root prepares a host-only quote build against the reviewed exact Reth candidate using an isolated, recorded source/dependency override. Adapt only required host snapshot/report plumbing, if any, in the maintained Raiko development worktree. Reuse the same native constructor, canonical inputs, source-bound ELF and original SP1 observations; no guest or SP1 rebuild/rerun is needed to reprice development inputs.
6. **Actual replay and fresh validation.** Requote all60 original canonical inputs; independently verify only the intended additive contribution changed, event counts and exact scaled uplifts, plus all original identities/resources/output. Freeze policy before opening new arithmetic/context/stack composition validation inputs that were not used for fitting; qualify their semantics and sample actual PGU separately.
7. **Historical sensitivity.** Native replay the same retained Mainnet cohort against parent/candidate, preserving both surfaces and original actuals/program identities. Historical actuals remain cross-program diagnostics where applicable. If the refinement materially changes the prior roughly2x Mainnet conclusion, discuss the measured tradeoff before promoting the candidate.

### Prospective validation panel

Freeze the 64-PGU candidate/model before opening these actuals. Add a separate24-case catalog with fixed128-site bodies and counts0/8192/196608 for eight patterns: DUP16/POP, double SWAP16, asymmetric full-width XOR, SHL255, SAR255, CALLVALUE/POP, TIMESTAMP/POP and NUMBER/POP. Keep17 fixed seed words and a memory-word counter. Arithmetic/shift warmups check expected results; all patterns check final seed preservation. Independently qualify actual contextual values against the constructed header/value, signed shift semantics and full opcode maps. Preserve all earlier60 requests and their metadata. This panel adds new shape/counts and opcode/context combinations; it is not used to select64. Exact input/program identity and true candidate native quotes precede actual sampling, with both estimate surfaces and all N0 controls retained.

## Operational constraints

- Keep Reth #260 and Raiko #284/#296 pending/do-not-merge; no automatic merge or deployment.
- Preserve primary dirty checkout and all immutable Atlas fixtures. New verified evidence gets a new fixture revision/PR; no historical asset rewrite.
- No guest build, arbitrary global discount adjustment, coefficient edits to unrelated models, new dependency installation or unnecessary repeated test runs.
- Reuse existing Cargo caches and quote-build source snapshots, one job at a time; disk currently has less than7GiB free. Stop bounded builds safely if space/memory limits cannot be maintained; do not delete user data to continue.
