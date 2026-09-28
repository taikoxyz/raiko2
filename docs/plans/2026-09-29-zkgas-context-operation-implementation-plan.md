# ZKGas Context Operation Calibration Plan

## Status

In progress.

This milestone closes the first ordinary-operation gap after typed storage. It is intentionally
limited to six environment and calldata opcodes that can be isolated without account-state,
memory-growth, return-buffer, halting, or CALL-wrapper semantics:

- `ADDRESS` (`0x30`);
- `CALLER` (`0x33`);
- `CALLVALUE` (`0x34`);
- `CALLDATALOAD` (`0x35`);
- `CALLDATASIZE` (`0x36`);
- `TIMESTAMP` (`0x42`).

The architecture remains the one in
[the calibration design](2026-09-26-zkgas-calibration-design.md). This plan does not alter the
historical opcode core, production ZKGas schedule, block limit, Boundless configuration, frozen
proposal corpora, or higher-layer derivation.

## Why This Is A Separate Campaign

The proposal trace already emits these opcodes as ordinary `static_raw_gas` events, but no sealed
controlled SP1 measurement exists for them. The production Unzen multiplier is schedule metadata,
not proving-cost evidence. The existing core can contribute only its sealed `common_dispatch` and
the measured control-opcode body.

The campaign therefore produces a separate content-addressed augmentation. V1 through V4 and the
original Osaka augmented core remain byte-for-byte immutable. Before this campaign is promoted, a
formula-only successor to the Osaka core corrects the historical `ISZERO`/`CLZ` recovery by applying
the sealed `body_scale` to their lab-basis relation slopes. That correction reuses the original raw
rows, requires no SP1 resampling, and produces operation coverage V5. This campaign records
provisional context models; only a later artifact with independently supported cross-ELF transport
may derive operation coverage V6 from V5.

The context-input change must not change the legacy opcode-lab wire format, public commitment, or
guest binary. `sp1_revm_opcode_lab.elf` retains the historical `OpcodeLabInput` contract;
`sp1_context_opcode_lab.elf` owns the new context-aware input and public commitment. Before any old
control body or `body_scale` is reused, the isolated legacy ELF must pass the existing non-fitting
Osaka compatibility-canary relation set and thresholds. Separately, measured `PUSH0` and `SWAP1`
remain anchored by the historical `09ebb08d76d3f461086b0cf4` absolute-probe evidence: the
same-revision `sp1_opcode_lab.elf` reruns that frozen probe and must pass 10% per-control APE and 5%
MAPE. The canary binds all three ELF identities and the common launcher.

The fixed legacy canary is not adaptive and may not expand its bound, refit a historical relation,
or execute the context ELF. It authorizes reuse only on the preserved legacy cost surface. A
separate context ELF can introduce a different SP1 marginal-cost surface even when both guests run
the same EVM microprogram. V1 therefore marks cross-ELF context-to-legacy cost transport
`not_evaluated`, requires transport evidence before candidate promotion, and keeps every context
result `candidate_eligible=false`. The campaign may still collect and seal non-candidate controlled
evidence; it must not silently treat the legacy canary as transport proof.

## Controlled Environment

Add a versioned `ContextOpcodeLabInputV1` with canonical fields for transaction calldata,
transaction value, and an optional block-timestamp override. Keep `OpcodeLabInput` byte-for-byte on
the legacy wire and retain its historical default transaction/block environment and public
commitment. The shared `raiko2-opcode-lab` context constructor is the only implementation of the
corresponding REVM `TxEnv` and `BlockEnv`; the context host tracer and context SP1 guest must use
that same constructor. The context binary input, workload identity, transaction-envelope identity,
and guest public commitment bind the resolved values.
Because timestamp is not part of a transaction envelope, identity evidence and trace/report joins
also carry a canonical block-environment digest; binding only the serialized backend input is not a
substitute for this explicit execution-envelope check.

REVM's existing `BlockEnv::default()` uses timestamp `1`. An omitted timestamp override must retain
that exact legacy environment. Explicit timestamp `0` is a distinct required scenario and therefore
cannot be represented by treating numeric zero as an omitted default. Identities and public output
bind the resolved effective timestamp.

Freeze these required scenario sets before measurement:

| Key | Required scenarios |
| --- | --- |
| `ADDRESS` | canonical benchmark target |
| `CALLER` | canonical benchmark caller |
| `CALLVALUE` | zero and nonzero value |
| `CALLDATALOAD` | empty, full in-range word, partial word, and fully out-of-range load |
| `CALLDATASIZE` | calldata lengths `0`, `1`, `31`, `32`, and `33` |
| `TIMESTAMP` | explicit zero and an explicit non-default, nonzero timestamp |

Each scenario uses fixed-footprint target/control microprograms, a fixed transaction gas limit,
three identical repeats, the existing frozen adaptive count prefixes, and an out-of-fit checkpoint.
The first round has generator bound `8` and counts `0, 1, 2, 4, 8`: `0, 1, 2, 4` are the first fit
prefix and `8` is its checkpoint. Only rejected relations advance through the existing
`8, 32, 128, 512, 2048` bounds; an accepted relation freezes immediately. The count-zero pair is
required to detect lane/input bias and cannot be omitted merely because positive counts fit well.
Target and control must have identical serialized environment, bytecode footprint, setup, cleanup,
and final stack height. Zero-input producers use measured `PUSH0` as the control. `CALLDATALOAD`
keeps a second dummy stack item and uses measured `SWAP1` after the same two-item setup, so both
lanes retain two words and execute one one-byte, three-raw-gas measured instruction before common
cleanup. `NOT` is forbidden as a recovery control because its sealed body is an explicit
dispatch-only approximation rather than an independently measured body.

## Parameter Recovery

Let `delta_lab` be the fitted target-minus-control marginal SP1 `proverGas` per event in the
controlled lab-body basis, `r_t` and `r_c` the executed raw EVM gas of target and control, `s` the
sealed legacy `body_scale`, and `b_c` the sealed legacy production-scaled body per raw gas of the
control opcode. The common dispatch and identical surrounding instructions cancel inside the
context guest, producing this provisional legacy-basis projection:

```text
delta_lab = r_t * lab_body_t - r_c * lab_body_c
b_t       = (s * delta_lab + r_c * b_c) / r_t
event_cost = common_dispatch + raw_evm_gas * b_t
```

The projection uses exact `Fraction` equations and canonical 80-digit `Decimal` serialization. It
must not combine an unscaled relation slope directly with a stored production-scaled control body,
subtract `common_dispatch` a second time, or apply `body_scale` again after storing `b_t`.
However, exact arithmetic does not prove that `delta_lab` from the context ELF shares the legacy
guest's SP1 marginal-cost scale. Until the cross-ELF bridge is supported, store the parameter basis
as `provisional_legacy_projection_unvalidated_cross_elf_transport`; do not use `b_t` in a production
overlay.

Every required scenario for a key must pass the existing signal, repeat, fit, residual, and
extrapolation gates. Recovered bodies across required scenarios must also pass the frozen 5%
consistency gate. If any required sibling is missing, rejected, or inconsistent, the entire key
remains unmeasured. Proposal rows cannot fit, choose, or repair a parameter.

## Artifact And Promotion Contract

The create-only result directory binds:

- the frozen campaign manifest and exact generated-fixture identities;
- a passed fixed legacy compatibility canary, the separate context ELF identity, and an explicit
  `not_evaluated` cross-ELF transport status;
- implementation revision, Unzen/Fusaka/Osaka version axes, launcher, and SP1 ELF/VK identities;
- every raw target/control row, adaptive decision, repeat, checkpoint, and fit result;
- the exact sealed core artifact and control-model parameters used by parameter recovery;
- the recovered per-key bodies and complete scenario evidence;
- source-file hashes and a canonical content identity.

Directory-only replay must regenerate fixtures, refit every relation, recover every parameter, and
recompute every hash without executing SP1.

Operation coverage V5 is derived from V4. The only semantic parameter changes are the recovered
production-scaled `ISZERO` and `CLZ` bodies. Because the registry is one content-addressed core,
every measured core-owned row must update its artifact reference and provenance to the corrected
successor; unsupported/precompile classification and side-effect ownership remain unchanged.
A future operation coverage V6 may be derived from V5 only after an independently supported
cross-ELF transport artifact makes the result candidate-eligible. That later step may change only
the six rows above from `explicitly_unsupported` to measured `static_raw_gas`. Until then, V5 stays
authoritative and all composite-overlay entrypoints must reject the provisional context result.

## Implementation And Verification Order

1. Add failing Rust tests proving the legacy JSON/bincode/public-commitment contract remains
   unchanged and testing the new context binary contract, canonical environment identity, shared
   host/guest construction, and environment-sensitive semantics.
2. Keep the legacy guest on `OpcodeLabInput`, implement `ContextOpcodeLabInputV1` and the shared
   context REVM constructor, and expose distinct `context-opcode-lab` and
   `context-opcode-identity` launcher stages.
3. Add failing Python tests for manifest validation, exact fixed-footprint controls, sibling
   rejection, provisional parameter recovery, tamper rejection, V4/V5 immutability, and rejection
   of V6/composite promotion without supported transport.
4. Implement generation, execution/resume, fitting, sealing, directory replay, V5 preservation, and
   the candidate/transport promotion gate.
5. Run focused Rust/Python checks, the complete opcode-gas suite, formatting, byte-compilation, and
   diff/path hygiene.
6. Build the current SP1 guest artifacts and independently review the complete source and artifact
   diff.
7. Run and seal the existing non-fitting compatibility canary against the preserved legacy ELF;
   bind, but do not execute or claim transport validation for, the separate context ELF.
8. Execute and seal the controlled campaign. If any key fails, preserve the rejected evidence and
   retain measured provisional models only for keys whose complete required scenario set passed;
   never weaken a gate after seeing results.
9. Do not re-seal the composite estimator while cross-ELF transport remains `not_evaluated`.
   Independently validate or replace that transport first; only a later candidate-eligible milestone
   may replay the same two ad-hoc Mainnet proposals.

## Deferred Families

The next family is memory/copy/output: `CALLDATACOPY`, `CODECOPY`, `RETURNDATACOPY`, `RETURN`, and
`LOG0..LOG4`, using typed copy/output length plus the sealed shared memory function.
`RETURNDATASIZE` requires a real prior-call return buffer. `EXTCODESIZE` and related account
operations require a separate warm/cold account-access model. CALL/CREATE wrappers and direct
precompiles remain later operation-layer milestones.
