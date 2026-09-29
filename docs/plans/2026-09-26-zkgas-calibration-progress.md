# ZKGas Calibration Progress

## Status

In progress.

Last updated: 2026-09-29.

This document is the execution-status ledger for the ZKGas calibration work. It records what is
currently proved, what is actively being changed, and which gate opens the next layer. It does not
replace the [architecture design](2026-09-26-zkgas-calibration-design.md), the
[execution plan](2026-09-26-zkgas-calibration-execution-plan.md), or the operational details in the
[opcode experiment README](../../experiments/opcode-gas/README.md).

Update this file only at a milestone boundary. Do not use it as a command log or copy generated run
output into it.

## Canonical Objective

Build a high-precision estimator from host-computable execution and state features to backend-native
ZK proving cost. V1 measures SP1 normalized `proverGas`. A later backend may independently calibrate
the same workload decomposition in its own native cost unit.

The target decomposition is:

```text
proposal_cost =
    proposal_startup
  + sum(block_base
      + state_trie_cost
      + sum(tx_base
          + native_transfer_cost
          + opcode_cost
          + precompile_cost
          + call_create_wrapper_cost))
```

`tx gas` in this project means the per-operation execution ledger and actual raw EVM gas. It never
means transaction `gas_limit`, and final receipt `gasUsed` alone is not sufficient because executed
work may later revert or reset.

The state/trie term is a separate architectural layer, not a cost assigned blindly to each
`SLOAD`/`SSTORE`. The first block-level model may keep fixed witness/trie/hash work in `block_base`.
Split a variable state/trie function only when controlled block residuals show that the coarse model
is insufficient.

## Version Identity

Keep these version axes distinct:

- Taiko fork and production ZKGas schedule: Unzen and `UNZEN_ZK_GAS_SCHEDULE`;
- Ethereum upgrade terminology: Fusaka;
- EVM execution specification used by REVM: `SpecId::OSAKA`;
- proving-cost backend for V1: SP1 normalized `proverGas`.

Raiko2 maps `TaikoFork::Unzen` to `SpecId::OSAKA`. Every new controlled fixture must therefore use
Osaka execution semantics while continuing to bind the actual exported Unzen schedule identity.

## Layer Progress

| Layer | State | Exit gate |
| --- | --- | --- |
| Experiment framework | Core scope complete | Preserve its identity and replay invariants |
| Opcode core | Complete at partial coverage | Preserve sealed 101-plus-2 augmentation |
| Remaining operations | Complete at frozen classification | Preserve the sealed coverage and ownership ledgers |
| Stateful storage execution | Promoted and replayed on two ad-hoc proposals | Preserve sealed typed features while closing the remaining operation families |
| State/trie | Coarse model accepted | Preserve the sealed holdout evidence; split only after a new predeclared experiment |
| Transaction | Declared approximation accepted and sealed | Preserve `5017` and its `0.002` materiality budget |
| Block | Fixed base accepted and sealed | Preserve the selected round-128 fixed-cost artifact |
| Proposal | Plumbing reviewed; two existing-fixture ad-hoc diagnostics opened | Promote reviewed operation families, then rerun the two diagnostics before opening frozen smoke rows |
| Other ZKVM backends | Future | Measure natively or validate an explicit bridge |

## Sealed Baseline

The current immutable baseline is derivation `3e1d97c461cd2ef9a40e6a02`, built from historical
calibration run `09ebb08d76d3f461086b0cf4`.

- Core status: `supported_core_submodel`.
- Modeled named opcodes: 101 of 150.
- Explicitly unsupported named opcodes: 49.
- Dynamic model: supported with exact rank 14 of 14.
- Structured opcode families: `EXP`, `KECCAK256`, `MLOAD`, `MSTORE`, `MSTORE8`, and `MCOPY`.
- `ISZERO` is unmeasured because its historical relation depended on dispatch-only `NOT`.
- `CLZ` is absent because the historical opcode guest used Prague execution semantics.
- Candidate eligibility: false.
- Production schedule, runtime configuration, block limit, and Boundless configuration changed: no.
- Final-validation proposals opened: no.

The baseline is historical evidence. Never rewrite it or claim that its 101 coefficients were
measured under the Osaka guest.

## Completed Milestone: Osaka Opcode Supplement

The active milestone is deliberately bounded:

1. Run the REVM opcode lab with `SpecId::OSAKA`.
2. Add `CLZ` to the controlled manifest and core inventory.
3. Give `ISZERO` and `CLZ` a one-opcode `SWAP1` control with matching stack shape and final stack
   height, so common dispatch cancels without depending on `NOT`.
4. Complete focused Python and Rust tests, build the SP1 guest artifacts, and independently review
   the complete change.
5. Execute a small compatibility canary for representative sealed baseline families. The canary is
   a reuse check, not coefficient fitting and not a full resample.
6. Sample only the `ISZERO` and `CLZ` formal relations under the Osaka guest.
7. Seal an augmented, non-candidate artifact that binds both the immutable 101-opcode baseline and
   the two supplemental relations. Its provenance must state that the old coefficients were reused,
   not remeasured.

Expected coverage after this milestone is 103 of 150 only if both supplemental relations pass all
predeclared gates and the compatibility canary shows no material baseline drift. Failure preserves
the 101-opcode baseline and records the failed supplement; it does not trigger an automatic full
resample.

Sealed result:

- implementation revision: `571dd487ffef8c3e158a376e13c9e159da7c936c`;
- Osaka calibration run: `51f71fde68f378842f872fc1`;
- SP1 REVM opcode-lab ELF SHA256:
  `a4d340812a54a36ce57cdd0f197843f43f67fd9ac7450acee2ec1f6e58eb9a56`;
- augmentation: `f945e67bb2c38c9c8ef50530`;
- augmented core artifact SHA256:
  `b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b`;
- compatibility canary: passed all eleven relations, with drift MAPE
  `0.00031986761894294409128641959166862087318487194524130119483505145697115354057520394`
  and maximum per-relation APE
  `0.00082987551867219917012448132780082987551867219917012448132780082987551867219916349`;
- supplemental relations: `opcode:0x15:canonical` and `opcode:0x1e:canonical` both accepted at
  generator bound 32, then solved exactly against the shared `SWAP1` control;
- solved static bodies per raw gas:
  - ISZERO: `13.299244356886983625148164851060802046871571495401800960152582501961195825989132`;
  - CLZ: `15.847288549616061142830834394507448970058426768208822511575420468918652979464447`;
- coverage: 103 of 150 named opcodes modeled, 47 explicitly unsupported, with all 101 historical
  models reused and zero historical coefficients remeasured;
- candidate eligibility: false;
- final-validation proposals opened: no;
- production table, schedule, runtime configuration, block limit, and Boundless configuration
  changed: no.

Task 2 implementation passed 31 focused Osaka tests, 91 candidate-report tests, and the complete
376-test opcode-gas Python suite with one existing opt-in skip. The same independent reviewer closed
all semantic-replay findings, and an independent tester reproduced directory-only replay, fully
re-sealed forgery rejection, guest-identity rejection, and path/symlink failure behavior. The live
supplement verifier and the sealed augmentation verifier both completed successfully.
An independent Task 3 reviewer replayed the four-file package and reported no material findings.
The independent tester reproduced the run-level verifier from a clean checkout at the frozen
implementation revision and replayed the sealed augmentation from the staged package. The run-level
verifier intentionally rejects later documentation edits; portable post-result verification uses
the self-contained augmentation verifier.

## Completed Milestone: Operation Coverage Ownership

The operation boundary is frozen in
`experiments/opcode-gas/manifests/operation-coverage-v1.json`, artifact SHA256
`fbb4817d50b04147d0d9c86a25c82324d5e36ce9d6acbf49c51dd165cf1905a3`.

- Execution coverage contains all 168 active named schedule entries exactly once: 97 sealed static
  opcode models, six sealed structured opcode models, 18 direct precompile bodies, and 47 explicitly
  unsupported opcode executions. The precompiles are classified by execution shape but remain
  `declared_unmeasured`; this milestone does not treat the current schedule multiplier as a measured
  proving-cost artifact.
- Side-effect ownership contains 13 declared events exactly once: three operation-wrapper, five
  state/trie, two transaction, and three block events.
- Emitted trace fields are distinguished from ownership boundaries that are not yet emitted. In
  particular, account/storage access, dirty-state update, final-trie update, witness/state input
  validation, and block context are `declared_not_emitted`; the manifest does not claim they were
  observed by the current trace crate.
- `SSTORE` remains an unsupported operation-execution key while storage access, dirty-state update,
  and final-trie update are separately owned by the state/trie layer. `CALL` likewise remains an
  unsupported operation-execution key. For a confirmed spawned `CALL`, the current inspector emits
  one fixed wrapper row that substitutes for the opcode raw-gas row; it does not emit and charge both
  body and wrapper. Executed child operations match normal execution coverage, while
  `child_execution` is only a derived grouping with zero additional charge.
- The manifest is replayed from the exported Unzen schedule, the sealed 103-opcode Osaka registry,
  its exact four-file Task 3 augmentation envelope, the named precompile inventory, and exact
  trace/design source hashes. The core reference is pinned to a repo-contained regular non-symlink
  file with exact canonical bytes. Validation rejects caller-resealed core forgeries, path or disk
  drift, unknown, duplicate, or missing execution/event identities, missing evidence, measured
  entries without an artifact, unsupported entries without a machine-readable reason, and
  whole-opcode ownership delegation.
- Operation charging selectors require a transaction-phase operation joined to exactly one started
  non-Anchor transaction. Unattempted transactions do not match `tx_base`; Anchor and system work
  remain mutually exclusive block-owned paths. Native-transfer work additionally requires the
  trace's committed-success predicate, and operation rows outside the frozen named opcode/precompile
  inventory fail closed instead of creating an unclassified execution key.
- Proposal execution remained closed. No sampling was performed and no production table, schedule,
  runtime configuration, block limit, or Boundless configuration changed.
- The first independent adversarial review found two source-replay and trace-selector blockers. Both
  were fixed. The updated focused suite passed 37 tests with one skip, and the complete opcode-gas
  Python suite passed 402 tests with one skip. A second review round found and then verified the
  native-transfer selector, frozen-key rejection, and authoritative CALL wording fixes; the final
  focused review suite passed 41 tests with one skip and reported no remaining findings. The fresh
  complete opcode-gas Python suite passed 406 tests with one skip.

## Blocked Milestone: Higher-Layer Fixed Costs

The bounded production-SP1 campaign stopped fail-closed at its predeclared maximum overhead bound.
It did not produce an accepted four-cost artifact, did not open state holdouts, and did not seal a
derivation.

Execution identity:

- implementation revision: `1155fb384da5c06a369b73da07e12728d162eb3b`;
- canonical run: `999b91b91fd693899d09fa53`;
- full run identity SHA256:
  `999b91b91fd693899d09fa53c0502c19e6c43d6f9a0ddcf4c38bd15ef9c0fccd`;
- production guest-launcher SHA256:
  `80f5d04b607ccc650c0e99714453b90c81d7d272af9e3bfc63f8e4b24c28ff53`;
- SP1 Shasta proposal ELF SHA256:
  `e32daf0bf9e981162c95c95e4004996a9be153357c503dd6373d647559b41bf1`;
- SP1 Shasta proposal VK SHA256:
  `f7ddfdf9434ef7a4569922cd54714dd39a182c945e4d37e65a51a4bb89e7f97e`;
- higher-layer manifest artifact SHA256:
  `f531201bd93f98ba20e51474ae4d64ae1f444bdaad027ea88362a47fb45e9809`;
- frozen operation-coverage artifact SHA256:
  `fbb4817d50b04147d0d9c86a25c82324d5e36ce9d6acbf49c51dd165cf1905a3`;
- augmented Osaka core artifact SHA256:
  `b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b`.

The controller published exactly the three authorized rounds:

| Bound | Decision | Raw rows SHA256 | Fit SHA256 | Root reason |
| ---: | --- | --- | --- | --- |
| 8 | `expand_next_round` | `9b6c838eb70b2a20c10e36240b115a29c69c5b4487a1cd13c827f18871fd29e2` | `514c8d18e75120847bd58d9ee96190361d1c4e2e0ba56e1f15a84d1eff6505af` | `exhausted_sweep` |
| 32 | `expand_next_round` | `05995ff07c08cd2035af7d50533cb89562bd39c63840901007dabcef4ea92874` | `4cb5374e5771014956b5ff4cae17014d89f68af4f71ba0324f69c3f4b29a1901` | `exhausted_sweep` |
| 128 | `terminal_failure` | `297d88799b069765481f9793645da504c507b14193677109cf760803d491140e` | `fc92696741c1b74503b4863f156936a6b3ad00164222058d8db26f14f637b3e5` | `exhausted_sweep` |

At bound 128, both tx-base cases passed the primary fit and checkpoint gates. Their checkpoint APEs
were respectively
`0.0013251328670122864495007550638558906913476686660355976150006060471772567880810233`
and
`0.0013714482628306657157669194327305215850772189025004586950608042066992932335457321`.
The rejected terminal fit computed tx-base
`172458.52776158196293291840525518983976157762777355659715869882250567609245941806`;
this value is evidence inside a rejected round, not an accepted fixed-cost artifact. The other three
fixed costs are absent:

- `native_value_transfer`: rejected on `primary_residual` at bound 128, then
  `exhausted_sweep`;
- `block_base`: not measured because the native-transfer dependency remained closed;
- `proposal_startup`: not measured because the native-transfer and block-base dependencies remained
  closed.

There is therefore no selected accepted round, no four-cost digest, and no higher-layer derivation
ID. All six frozen state pairs remained closed and have no result:
`witness_topology_1`, `witness_topology_8`, `witness_topology_32`, `dirty_accounts_2`,
`dirty_accounts_8`, and `dirty_accounts_32`. No coarse-state verdict exists. Proposal evidence also
remained closed.

The profiling-enabled release build and fresh preparation passed, and the bounded overhead command
completed normally with the explicit terminal result. Fitting, state execution, finalization,
verification, and sealing were not run because their predecessor gate did not pass. The earlier
identity-only run `72d6408bcabf192c420d8a31` is preserved as an aborted integration attempt; it
identified a JSON-versus-bincode GuestInput hash mismatch. Fix `1155fb38` aligned the Rust workload
identity with the production backend bytes and passed independent review and behavioral testing
before the canonical run above. No opcode coefficient was refit, no final proposal was opened, and
no production schedule, runtime configuration, block limit, or Boundless configuration changed.

## Accepted Design: Conservative Native-Transfer Approximation

The terminal campaign remains immutable rejected evidence. It is not relabeled as an accepted
four-cost artifact. Its diagnosis established that the native EOA transfer path is separate from
contract/EVM execution, but the current positive-versus-zero matched control also contains different
signature-recovery work and positive-transfer balance/final-trie work.

The next campaign will use a frozen conservative approximation rather than block higher layers on
this small mixed signal:

- `native_value_transfer = 5017 proverGas` per committed native EOA transfer;
- coefficient source: the maximum observed per-transfer delta over counts
  `1, 2, 4, 8, 16, 32, 64, 128` in run `999b91b91fd693899d09fa53`;
- status: `declared_approximation`, never an accepted OLS coefficient;
- materiality gate: maximum
  `abs(5017 * count - observed_native_delta) / target_total_prover_gas <= 0.002`;
- observed count-128 materiality from the source evidence:
  `219576 / 181155333 = 0.0012120868669099573...`;
- EVM call wrapper, opcode, and precompile work remain separate and are not included in this value.

The new policy preserves all provenance, repeat, execution, positive-signal, and ownership checks.
It changes only the small native relation's quality gate. Block and startup cases are blocked only
by dependencies with nonzero deltas, so the native approximation does not prevent those zero-native-
delta cases from fitting. A new clean campaign identity must validate the already frozen `5017` and
`0.002`; it may not reselect either value after results are visible.

State holdouts remain closed until the policy is implemented and the three ordinary fixed terms plus
the native materiality gate pass. Pairwise state predictions cancel the approximation where lane
native counts are equal. A later holdout failure remains evidence for a predeclared state/transaction
split; it cannot tune this approximation. Final proposal validation and production changes remain
closed.

## Completed Milestone: Higher-Layer Fixed Costs And Coarse State Holdouts

Fresh production-SP1 calibration run `91d461f0a817446a80223798` accepted the frozen native-transfer
approximation, all three ordinary fixed costs, and all six coarse state holdout pairs. It sealed
portable derivation `3e4de6eecb5e92aa59a6a4b9`. This result did not open proposal inputs and did
not change a production table, schedule, runtime configuration, block limit, or Boundless behavior.

Execution identity:

- implementation revision: `29497a4c8f8bf851e5d572924c6bf616d727e37b`;
- full run identity SHA256:
  `91d461f0a817446a802237987c08f1290c571d19b245e7655084600ba05b68ff`;
- production guest-launcher SHA256:
  `80f5d04b607ccc650c0e99714453b90c81d7d272af9e3bfc63f8e4b24c28ff53`;
- SP1 Shasta proposal ELF SHA256:
  `e32daf0bf9e981162c95c95e4004996a9be153357c503dd6373d647559b41bf1`;
- SP1 Shasta proposal VK SHA256:
  `f7ddfdf9434ef7a4569922cd54714dd39a182c945e4d37e65a51a4bb89e7f97e`;
- higher-layer manifest artifact/file SHA256:
  `a748f12c0db6753d860a59141af747240123e28b7f62e32bd5ff2cfb0b4a792f` /
  `250f58dd1b9accc21b300bd859942fc66865936316a91619d35672b9964e70aa`;
- operation-coverage artifact/file SHA256:
  `35fd25a7878dd407522ab676c8a9965c663c6885e3190df0fd51f6c497b84e77` /
  `75fec3c4307c59539cc6180e6511dd7fc1111197746c6b0abd9a872902c665a3`;
- augmented Osaka core artifact SHA256:
  `b66d7951bfa416810f93319f99c30ce91969026ee9fa3a402a74cbc2214f7e8b`.

The bounded overhead ledger is:

| Bound | Decision | Raw rows SHA256 | Fit SHA256 | Root reasons |
| ---: | --- | --- | --- | --- |
| 8 | `expand_next_round` | `7541b8927fd734597b399361fe1568704b5fedda44b1984535857cbbe113b4ba` | `24cdcd7f015fdd52c90d760a2a8efcc1eb56c35c53b76fb17e4675a990614cbe` | `exhausted_sweep`, `native_approximation_requires_bound_128` |
| 32 | `expand_next_round` | `06936504f01d7a097d17f3a1127c1c087473b731fe64ba62487b670e05e523ec` | `f08eae73d042afc2b5d80019c864741f1bcd69addecf2505e7a70ff982ec3deb` | `exhausted_sweep`, `native_approximation_requires_bound_128` |
| 128 | `accepted` | `32fb52656c98b1588c50f8df7a1cb2836c8a7f7c721d681faf15994c4a4db801` | `027c23e2c7af3fdd24ba155376112285715fda7f7597541806c9a2ca89ae49fb` | none |

The frozen coefficient remained exactly `5017`, the materiality budget remained exactly `0.002`,
and the eight whole-guest materialities were:

| Native count | Materiality |
| ---: | ---: |
| 1 | `0` |
| 2 | `0.000007554867839990185077431617738608631482351357724047267423142344976466932079703391` |
| 4 | `0.000027876573185804334126037224379546367388020983484248833873882372791251510513641118` |
| 8 | `0.000023787124289764379831303436342997830449570285880434646719469216616446077115798266` |
| 16 | `0.00013528094442461295455196570719268505132454650506160174283064638202377757539606129` |
| 32 | `0.00027000102029296007402087728646646224453107197554341421320801122870611113292575235` |
| 64 | `0.00046191126123284270273615019917317454981086970405520195032831874844199106833571755` |
| 128 | `0.001212086866909957323751545310565049718961351250973108255112754533149736199044165` |

The maximum occurred at count 128 and remained below the budget. The accepted fixed model has
status `accepted_with_declared_approximation`, rank four, selected round 128, and digest
`b2f63b3c5624009b6116de095ea9d1e8607a1bfc8372578136c1fb4e2b535587`:

- `tx_base = 172458.52776158196293291840525518983976157762777355659715869882250567609245941806`,
  status `accepted`;
- `block_base = 2507390.6829493087557603686635944700460829493087557603686635944700460829493087559`,
  status `accepted`;
- `proposal_startup = 156283811.55316990026277317213377793503403626187735746133275705611870107900446154`,
  status `accepted`;
- `native_value_transfer = 5017`, status `declared_approximation`.

The state campaign produced exactly 36 successful execution rows, SHA256
`bf2b836e20a305eb8ec0b280568eea7f59f1cbb9e1a2219c9369e5117196e141`. Every predeclared pair was
accepted: `witness_topology_1`, `witness_topology_8`, `witness_topology_32`, `dirty_accounts_2`,
`dirty_accounts_8`, and `dirty_accounts_32`. The aggregate verdict is `coarse_model_accepted`; the
largest target APE was
`0.0091781895650865294983267566242553804960841536557328913471317871584700116890910893`
on `dirty_accounts_32`, below the frozen `0.1` gate.

Portable derivation identity SHA256 is
`3e4de6eecb5e92aa59a6a4b9d380765b94ddf9b1b8198479659393a878694f24`. Its exact files are:

- `identity.json`: `b68c892a25cec824cc5805b9f20baee163287caa91b0d146261e4497b042391e`;
- `overhead-evidence.json`: `e990deed7cf0e04153e716165e67ec1028c5c3479e8e99cda0d8f9872c826979`;
- `state-holdout-evidence.json`: `1004cb82af8a67fe9a3794044bc8b88f62a08fc095d528454940745c83690ec6`;
- `higher-layer-calibration.json`: `e06e90546f48b59f52670ceb320c63a5da64e910c7154efc95bafc4fa5579a54`.

Both live-run semantic replay and directory-only portable replay returned
`coarse_model_accepted`. Fresh validation passed 68 higher-layer Python tests, the complete 476-test
opcode-gas Python suite with one existing opt-in skip, and all 27 controlled-workload Rust tests.
The controller's independent post-commit replay and adversarial review remain the next gate before
this evidence is used to open any proposal input.

## Implementation Checkpoint: Proposal Integration-Smoke Plumbing

The Task 4 implementation adds the missing local proposal-validation path without opening proposal
evidence:

- `guest-launcher --stage proposal --proof-type sp1 --mode execute --sp1-prover local
  --sp1-execution-engine gas-estimator` loads the production proposal ELF, applies the frozen SP1
  gas-estimator chunk threshold `134217728` and slot count `2`, and emits GuestInput identity,
  public output, exit status, `proverGas`, primary metric, and execution diagnostics in one report;
- proposal gas estimation still rejects proof mode, a network prover, aggregation, and an alternate
  single ELF;
- `run-proposal --proof-type sp1` explicitly selects this engine and rejects a report unless its
  proposal/execute identity, canonical chunk parameters, GuestInput/public-output join fields, exit
  status, positive gas, and primary metric are valid;
- `prepare-corpus` discovery now passes L1 network `hoodi` for `taiko_hoodi` and `ethereum` for
  `taiko_mainnet` rather than relying on the discovery script's default.
- Smoke preparation accepts only Hoodi `79852` and Mainnet `38261`; normalized output keeps purpose,
  network, proposal ID, fixture digest, and workload identity, and non-SP1 smoke labels fail closed.
- Proposal reports bind the exact production proposal ELF and executing launcher digests. The
  launcher rejects `RAIKO2_GUEST_ELF_DIR`, hashes `/proc/self/exe` on Linux, and Python rejects wrong
  or mid-run-changing artifact hashes.
- GuestInput identity parsing and hashing use one byte read. Native trace and SP1 execute then consume
  the same read-only staged snapshot, so original-path mutation cannot change the executed workload
  after smoke verification.

No proposal execution, RPC acquisition, coefficient tuning, production schedule/config change, or
corpus publication occurred in this implementation checkpoint. Hoodi proposal `79852` and Mainnet
proposal `38261` are frozen only as the later integration-smoke pair and are disjoint from the still
unopened final corpus. The exact post-review smoke sequence is documented in the experiment README.

## Diagnostic Milestone: First Composite Proposal Replays

Task 4 passed independent review at implementation revision
`c93aac09b9a280b75110e0a4bc0bf975855da836`. The final reviewer reported no unresolved findings
after the frozen-smoke identity, exact ELF/launcher provenance, GuestInput and executable TOCTOU,
and normalized-input compatibility fixes. The reviewed release launcher SHA256 is
`f77bfed34a085961332dee6741397c2c3477831dae4735624193433ada3c32cc`; the production SP1 proposal
ELF remained
`e32daf0bf9e981162c95c95e4004996a9be153357c503dd6373d647559b41bf1`.

Composite estimator artifact
`6431ef06ace0ae748551b6d42312690b692322e1f28276460c71b79ddb7515f7` sealed and replayed exactly
at that revision. Two existing repository fixtures were then executed as `purpose=ad_hoc`
diagnostics. They are not substitutes for the frozen Hoodi `79852` and Mainnet `38261` integration
smokes, are not final-validation rows, and did not tune any coefficient:

| Network / proposal | Blocks | Actual proverGas | Modeled subtotal | Modeled / actual | Operation-count coverage | Raw-gas coverage | Result |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| Mainnet `23077` | 192 | `2047710602` | `651114652.98548710145385022502878729059677218985258403756560922755473186873126614` | `31.7972%` | `97.5691%` | `20.3116%` | `insufficient_coverage` |
| Mainnet `7857` | 192 | `1492313639` | `646682264.08691615025121578174724959606716717129236473771197697086806683189302328` | `43.3369%` | `96.3574%` | `11.1801%` | `insufficient_coverage` |

Both exact SP1 report joins passed. The estimator correctly withheld `predicted_prover_gas` and
APE because coverage was incomplete. The shared fixed contribution was approximately `637.7M`
proverGas: one proposal startup plus 192 block bases. Non-Anchor transaction base and modeled
operation work contributed only the remaining small subtotal. The 192 Anchor transactions produced
`2353267` block-owned operation rows in each fixture and were excluded from operation charging, so
the result did not double-count Anchor execution.

The gap shape was consistent across both workloads. SLOAD (`0x54`) and SSTORE (`0x55`) dominated
uncovered raw EVM gas: respectively `393000` and `355500` in proposal `23077`, and `455900` and
`280000` in proposal `7857`. The remaining material gaps were LOG variants, EXTCODE/account access,
CALL-family wrapper rows, and direct precompiles. This means high operation-count coverage is not a
sufficient completion signal. It does not yet prove that the roughly `1.40B` / `0.85B` difference
is operation cost: state/witness/trie and other higher-layer work remain possible residual owners.

Frozen smoke acquisition remains operationally blocked in this checkout. Public Taiko RPCs support
proposal discovery but returned JSON-RPC `-32601` for `debug_executionWitnessForTxList`; the
deployment's dedicated witness RPC is not reachable from this machine. The existing Hoodi fixture
for proposal `17462` was also rejected rather than reused: it records
`last_anchor_block_number=0`, while the current guest derives `2668320`. Its native trace was marked
failed and no estimator result was produced.

## Completed Milestone: Stateful SLOAD/SSTORE Execution

Formal SP1 sampling selected the predeclared typed storage model. The sealed artifact is derivation
`64065fa462311bdc1848e9d0`, produced from calibration run `e96fa5d0372dbb29cc9f606a` at
implementation revision `6e188fe00d15815190535cb1bb739198ecbe2fed`.

- production-path guest launcher SHA256:
  `f7e40fa7b820516418eb4bd80afd24b3a53705171dbbd9ffa3d1b44bd105351a`;
- SP1 REVM opcode-lab ELF SHA256:
  `ca7fd7f79189a382bbdd50d5a565b27e8ea382fe5382bb43bd43d994d8bed259`;
- SP1 REVM opcode-lab VK SHA256:
  `8d28d6949bab2e52b1711d015311df3e0a90dafd55fc6fa4b1be10052db42a50`;
- campaign: 40 frozen scenarios, 564 target/control pairs, 1,128 rows, three repeats, and 376
  distinct canonical guest inputs;
- terminal row-ledger SHA256:
  `f8a3977df12e461426dc48daeaa974d830e3e86e1ed9404e54a7d41856e9bd34`;
- all 18 primary low-value/low-slot scenarios passed their predeclared fit, noise, repeat,
  holdout, and checkpoint gates;
- all 22 high-limb value/slot diagnostics remained below the predeclared 10% model and
  low/high-consistency limits.

`M_fixed` and `M_access` were rejected. Their maximum holdout APE was respectively about 50.97%
and 46.12%; access class alone cannot represent SSTORE execution. `M_typed` was the first eligible
model in the frozen selection order:

| Typed parameter | SP1 proverGas per event |
| --- | ---: |
| SLOAD warm body | `3957.51520727213202200418390456906258` |
| SLOAD cold modifier | `-621.19690860215053763440860215053763` |
| SSTORE no-op branch | `4474.71437431588060011417067340115379` |
| SSTORE set branch | `4658.38204904706339581309540458394949` |
| SSTORE clear branch | `4642.02653829437522377008465189577744` |
| SSTORE reset branch | `4669.13204904706339581309540458394949` |
| SSTORE dirty-rewrite branch | `2145.59428022985909473782658737964841` |
| SSTORE restore-original branch | `2147.89468345566554635072981318610002` |
| SSTORE cold modifier | `-625.49489247311827956989247311827957` |

The negative cold modifiers are measured model terms, not a claim about protocol gas or total state
cost. They were independently present for SLOAD and SSTORE and differed by only about `4.298`
proverGas. They may reflect the exact REVM/SP1 warm-path implementation under the frozen
target/control envelope. Promotion must preserve the measured typed formula and must not replace it
with an unsigned generic “cold surcharge.”

`M_typed` reached maximum marginal-signal APE of about `0.2714%` on the untouched count-32
holdouts, `0.2068%` on the count-64 extrapolation checkpoints, and `4.0451%` across high-value and
high-slot diagnostics. The worst high-limb case was cold high-value SSTORE clear. No result-driven
count expansion or coefficient tuning occurred.

This artifact measures stateful REVM execution, including storage execution, journal updates, and
result-state construction. It deliberately excludes witness materialization, persistent dirty-state
commit, trie hashing, and final-state-root construction. The artifact remains
`candidate_eligible=false`, `proposal_validated=false`, and
`production_registry_modified=false`: the selected model cannot be promoted through the current
proposal trace because that trace does not expose reliable access warmth and original/current/new
storage relationships.

The formal run, verification-only campaign replay, sealing, and directory-only sealed-result replay
all completed successfully. Sealed replay regenerated all 376 fixtures and host-native semantic
identities while executing the SP1 guest zero times. No proposal was opened and no production
schedule, table, runtime configuration, block limit, or Boundless configuration changed.

Fresh root verification passed the complete opcode-gas suite (`576` tests with one opt-in skip),
Python byte-compilation, SP1 artifact provenance, and staged-diff checks. Independent adversarial
review reproduced the negative cold contrasts and exact model/gate decisions with no material
finding. Independent behavioral verification recomputed all nine typed parameters and 18 nuisance
intercepts using exact rational arithmetic, checked all 564 pair joins, observed 376 host-native
identity replays and zero SP1 executions, and confirmed that sealed-file hashes did not change.

## Completed Milestone: Typed Storage Promotion And Ad-Hoc Replay

The schema-3 operation trace now classifies completed storage execution from REVM journal state:

- `SLOAD` records exact warm/cold access;
- `SSTORE` records warm/cold access plus the frozen `noop`, `set`, `clear`, `reset`,
  `dirty_rewrite`, or `restore_original` branch;
- stale transaction IDs, access-list warming, nested CALL/DELEGATECALL storage owners, later frame
  or transaction reverts, and outer Taiko zkGas-limit failures have live-inspector coverage;
- malformed stacks, static-context rejection, stipend/dynamic-charge halts, missing post-state,
  uncalibrated dirty no-ops, and impossible dirty+cold inputs fail closed as explicit feature gaps.

The schema-2 review-only composite estimator now binds all nine files of sealed derivation
`64065fa462311bdc1848e9d0`, imports only its exact `M_typed` vector, and applies the storage terms as
absolute event costs without adding the core registry's common-dispatch term again. The historical
opcode registry remains unchanged. A new `operation-coverage-v4.json` preserves V2 and V3
byte-for-byte while refreshing current schema-3 trace-source provenance; the estimator overlay
promotes only `opcode:0x54` and `opcode:0x55`, leaving the other 166 coverage rows unchanged.

Pre-seal verification passed:

- `cargo test -p raiko2-zkgas-trace`: 6 unit, 31 inspector, and 9 reconstruction tests;
- complete opcode-gas Python suite: 581 passed, one opt-in skip, and 821 subtests;
- release `guest-launcher` build and the real Rust-to-Python stateful admission round trip;
- Rust formatting, Python byte-compilation, path hygiene, and diff checks;
- independent adversarial review after fixing full sealed-source validation and publication/source
  overlap guards;
- independent behavioral recomputation of all nine parameters, all 12 valid typed predictions,
  both invalid-input gaps, the exact coverage delta, and V2/V3/V4 provenance.

Implementation commit `e59ed1f6` was independently reviewed, committed, and pushed before sealing.
Clean-tree sealing produced review-only composite estimator
`43cdd0adc466743bbb4a2e3bea3e9b7e46618fb36ca948f0d52a090f804d60b8`; its canonical
`estimator.json` SHA256 is
`30f906ccf0b423c22ae0cb9710f6459821c1fdba2a568c931b99014d2c294c2a`. Exact directory replay
passed at the sealed implementation revision.

The two existing Mainnet fixtures were then rerun once through the schema-3 trace and the sealed
estimator. Proposal `23077` used the repository fixture; proposal `7857` used byte-identical local
fixture copies with raw JSON SHA256
`1471402130b15a0c15ab6f60fdf88efc702529666ef710d82928aace6686f607`. Neither result changed a
coefficient or entered the frozen integration-smoke or final-validation corpora.

| Proposal | Actual proverGas | Modeled subtotal | Modeled / actual | Operation-count coverage | Raw-gas coverage | Typed-feature coverage | Storage events / contribution | Result |
| ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `23077` | `2047758068` | `652536157.63976242073999746184419183860460314671167493544607810593120040856920075` | `31.8659%` | `97.9558%` | `65.5683%` | `100%` | `380` / `1421504.654275319286147236795` | `insufficient_coverage` |
| `7857` | `1492798702` | `647928684.87258107245161331278214983980544996787489959294938700912729756684222137` | `43.4036%` | `97.3061%` | `74.8810%` | `100%` | `349` / `1246420.785664922200397531019` | `insufficient_coverage` |

Both schema-3 parity gates and exact SP1 report joins passed. The typed storage layer closed every
observed SLOAD/SSTORE feature: no storage feature gap remained. The remaining gaps were 1844 ordinary
opcode, 132 confirmed spawn-wrapper, and 33 direct-precompile events for `23077`; for `7857` they
were 911, 52, and 28 respectively. The dominant ordinary gaps were `CALLDATALOAD`, `CALLDATASIZE`,
`CALLER`, `CALLVALUE`, `CALLDATACOPY`, `RETURNDATASIZE`, `RETURNDATACOPY`, and `RETURN`, followed by
LOG/environment/account-access entries. This observed ordering is diagnostic only; it may select the
next predeclared family but cannot repair the sealed storage coefficients.

The production zkGas registry, runtime configuration, block limit, Boundless configuration, frozen
integration smokes, and final proposal-validation corpus remain untouched.

## Completed Milestone: Context Opcode Isolation And Non-Candidate Sampling

The first context campaign attempt correctly stopped at the fixed legacy compatibility canary before
sampling any context coefficient. Calibration `d9b45369e540fb6606429fda` used the context-expanded
legacy guest and reproduced a material `JUMPI` drift: its bound-8 checkpoint APE was about `15.26%`,
above the frozen `10%` gate. `EXP` and `KECCAK256` still passed, so accepting the whole legacy table
or expanding only the failed canary relation would have been result-driven reuse. The canary remains
fixed at its historical program identity and is not an adaptive campaign.

The failure was repeatable even though the EVM program projection and raw EVM gas were unchanged.
A diagnostic default-context fast path reduced neither the direction nor the material size of the
drift (`JUMPI` checkpoint APE remained about `16%`) and was reverted. The context-expanded binary
input/public commitment had changed the SP1 guest cost surface itself; this is not evidence that
`JUMPI` changed under Osaka and must not trigger a one-relation resample.

The implementation was split into three explicit artifacts in one calibration identity:

- `sp1_revm_opcode_lab.elf` preserves the legacy `OpcodeLabInput`, default environment, and public
  commitment and is the only guest used by the fixed 11-relation reuse canary;
- `sp1_opcode_lab.elf` remains the independent historical PUSH0/SWAP1 absolute control guest;
- `sp1_context_opcode_lab.elf` owns `ContextOpcodeLabInputV1` and the new context campaign, with
  dedicated `context-opcode-lab` and `context-opcode-identity` launcher stages.

The legacy canary can authorize historical-table reuse only on the preserved legacy cost surface.
It cannot prove that a marginal coefficient from a different context ELF shares that scale. Until
an independent cross-ELF bridge is evaluated, the context artifact records transport status
`not_evaluated`, requires transport evidence before candidate promotion, and remains
`candidate_eligible=false`.

Calibration `5d3963807dc0282307b5e5f6` completed that bounded context campaign and sealed portable result
`79dcfe2d5be2c3432987a671`:

- execution revision: `7c71c7eed8be9979983cee12854ebe7a0ca87dbb`;
- analysis/seal revision: `3a43ceda8da6f7450aa683bced739d5e301f5923`;
- result identity SHA256:
  `79dcfe2d5be2c3432987a671f92ae94b44361c4a0ae914218c149efe7eaa8734`;
- result artifact SHA256:
  `b5c1e0efdf22d6c0bb1fef07e0a42785af6d49ae68ebf2c36fafb6399b541675`;
- compatibility canary artifact SHA256:
  `3f374d7c040edb6ba50f23c140ed8d4174e8da2305d163de353f9ed9d688f080`;
- deterministic adaptive archive: 3,708 formal rows, `970302501` uncompressed bytes,
  `24056930` compressed bytes, file SHA256
  `d24a6915e2a894e97e8f1c1d98ac471ad3423cc143d34d8dfff0cff2922d3219`;
- terminal evidence: 1,128 rows across all 15 frozen scenarios.

The adaptive controller opened only the frozen bounds `8`, `32`, `128`, `512`, and `2048`. Four
scenarios passed without result-driven threshold changes:

| Scenario | Selected bound | Slope (`proverGas` per repeated operation) | Checkpoint APE |
| --- | ---: | ---: | ---: |
| `caller_canonical` | 8 | `64.642857142857...` | `2.506%` |
| `calldataload_in_range` | 32 | `38.315860215053...` | `7.275%` |
| `address_canonical` | 128 | `71.419449723956...` | `0.109%` |
| `calldataload_partial` | 128 | `99.204407638700...` | `1.174%` |

Required-sibling completeness permits only `opcode:0x30` (`ADDRESS`) and `opcode:0x33` (`CALLER`)
to enter the sealed model inventory. `CALLDATALOAD` remains unmeasured as a production key because
its empty and out-of-range required siblings failed even though its in-range and partial scenarios
passed. `CALLVALUE`, all five `CALLDATASIZE` length scenarios, both remaining `CALLDATALOAD`
scenarios, and both `TIMESTAMP` scenarios reached bound 2048 without satisfying the frozen static
relation gates. This is evidence that the current scalar relation is insufficient for those
scenarios; it is not evidence that the opcodes cost zero or are unreachable.

The first materializing sealer exhausted memory because 970 MB of JSONL was copied into several
Python object graphs. Schema 2 now stores every formal row in a deterministic gzip stream and keeps
only compact decisions/fits in JSON. Directory replay re-admits and refits every archived row with
bounded reads. Because that derivation fix followed execution, provenance now records the original
execution revision separately from the later clean analysis revision; live sampling still requires
the checkout to match the frozen execution revision exactly. Legacy schema-1 result replay remains
byte-compatible.

The unmocked seal and `verify-result` both completed. Fresh root validation passed all 49 context
tests; independent review closed two schema-1 compatibility findings; independent behavioral
verification replayed the real 970 MB campaign, exercised non-local-revision, manifest-tamper, and
dirty-checkout failures, and passed all 49 tests plus 21 subtests. No proposal was opened, and no
production registry, schedule, runtime configuration, block limit, or Boundless configuration
changed. Composite resealing and proposal replay remain closed.

## Next Gate

Evaluate the context-to-legacy cross-ELF transport independently of the opcode fits. The bridge must
use predeclared common controlled workloads and may validate or reject transport; it may not tune the
four visible context slopes after seeing them. If transport is supported, promote only the complete
`ADDRESS` and `CALLER` keys. If it is not supported, keep the context result diagnostic and measure
those keys directly on the destination cost surface.

For the eleven failed scenarios, do not increase the repeat count beyond 2048 or weaken the gates.
Use their sealed residual shapes to predeclare the next smallest model family: explicit context-value
classes for `CALLVALUE`/`TIMESTAMP`, and length/range terms for calldata operations. A successor
campaign must retain required-sibling completeness and exact event matching. Only after an
independently reviewed transport decision and operation-family successor may the composite estimator
be resealed and the same ad-hoc block/proposal diagnostics rerun; those diagnostics still may not
repair coefficients.

After this context family, close the remaining ordinary returndata/copy/LOG/EXTCODE keys before CALL
wrappers and direct precompiles. Reuse sealed controlled evidence where its execution semantics and
model inputs are sufficient; otherwise predeclare the smallest additional controlled campaign.

The frozen Hoodi `79852` and Mainnet `38261` integration smokes remain required once a
witness-capable RPC or immutable GuestInputs are available. Record their exact trace/report joins,
predictions, actual `proverGas`, APE, per-layer contributions, coverage, and gaps without changing a
coefficient. The final 60-row corpus remains unopened.

## Next Milestones

### 1. Close Operation-Layer Coverage

Inventory the remaining opcode/precompile execution keys separately from their emitted side-effect
events. Do not assume all remaining execution keys need independent scalar measurements. The valid
execution outcomes are:

- a static raw-gas relation;
- a typed opcode-specific cost function;
- a precompile or fixed wrapper-event model;
- unreachable or inactive under the frozen fork identity;
- explicitly unsupported with measured coverage reported later.

Maintain a separate exact-owner ledger for wrapper, state/trie, transaction, and block events. An
SSTORE or CALL may have an operation execution model and separately owned side effects; never assign
the whole opcode to one higher layer. Seal both ledgers before fitting transaction or block residuals.

### 2. Calibrate State/Trie And Transaction Costs

Start with the smallest controlled feature set: fixed block state work, started non-Anchor
transaction count, and native value-transfer count. Keep opcode execution and final trie work in
different ownership domains. Add unique-access, witness-node, dirty-state, or hash/update features
only if the frozen coarse model fails.

### 3. Calibrate Block And Proposal Costs

Fit block base and holdouts from controlled production-guest fixtures. After every lower component,
formula, threshold, and coverage rule is sealed, open the fixed proposal corpus exactly once for
final validation. Proposal residuals may motivate a new experiment but may not repair the current
candidate.

### 4. Extend To Other Backends

Keep SP1 `proverGas`, SP1 instruction count, and RISC0 cycles as different units. A future RISC0 run
executes the same controlled workload identities and may produce a backend-specific table. Only an
independently validated bridge may transport a cost vector between backends.

## Current Non-Goals

Until the remaining operation layer and every higher layer are sealed, do not:

- resample all 101 historical opcode coefficients;
- run the final proposal corpus;
- update the production multiplier table or integer scale;
- change the production block ZKGas limit;
- update Boundless quoting;
- treat SP1 instruction count as proving cost;
- attribute state finalization or revert behavior to an isolated opcode coefficient.

## Update Checklist

At each milestone update, record:

- exact immutable run or derivation IDs;
- implementation revision and guest ELF identity;
- supported, unsupported, and unmeasured coverage counts;
- validation commands that actually passed;
- independent review outcome;
- whether any proposal result was opened;
- whether any production code, schedule, or configuration changed;
- the single next gate and any material blocker.
