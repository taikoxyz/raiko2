# ZKGas Calibration Progress

## Status

In progress.

Last updated: 2026-10-02.

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
| Context-sensitive operations | Fully evaluable review-only candidate sealed | Freeze the controlled-block split before executing calibration |
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
- On 2026-10-02, bounded witness proof-history had expired the original smoke pair. Smoke preparation
  now accepts only preflight-validated Hoodi `80907` (L2 `20889658..20890035`, L1 inclusion
  `3736986`, last anchor `3736925`, 378 blocks) and Mainnet `39339` (L2
  `12149863..12150054`, L1 inclusion `26104557`, last anchor `26104488`, 192 blocks). Both were
  confirmed outside the final 60; normalized output keeps purpose, network, proposal ID, fixture
  digest, and workload identity, and non-SP1 smoke labels fail closed.
- Proposal reports bind the exact production proposal ELF and executing launcher digests. The
  launcher rejects `RAIKO2_GUEST_ELF_DIR`, hashes `/proc/self/exe` on Linux, and Python rejects wrong
  or mid-run-changing artifact hashes.
- GuestInput identity parsing and hashing use one byte read. Native trace and SP1 execute then consume
  the same read-only staged snapshot, so original-path mutation cannot change the executed workload
  after smoke verification.

No proposal execution, RPC acquisition, coefficient tuning, production schedule/config change, or
corpus publication occurred in this implementation checkpoint. The original smoke pair later expired
from bounded witness proof-history; on 2026-10-02, preflight validation replaced it with Hoodi
proposal `80907` and Mainnet proposal `39339`, both disjoint from the still unopened final corpus.
The exact post-review smoke sequence is documented in the experiment README.

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
diagnostics. They are not substitutes for the frozen Hoodi `80907` and Mainnet `39339` integration
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
It cannot prove that a marginal coefficient from a different context ELF shares that scale. The
context artifact therefore records transport status `not_evaluated` and remains
`candidate_eligible=false`. The project will not assume a scalar cross-ELF bridge: the old ELF stays
the immutable source of its existing registry, while the context ELF is discovery evidence for
function shape. Final coefficients for newly modeled operations will be fitted independently with
controlled production-guest workloads.

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

## Completed Milestone: Production-Guest Context Calibration Rejected

The frozen production-guest campaign completed from clean calibration revision
`dda214e385ee9608ad88ae18eefe3b7111224f45` and sealed the immutable result directory
`experiments/opcode-gas/derivations/a41befb63e663890896ba67d`:

- result ID: `a41befb63e663890896ba67d`;
- full result identity SHA256:
  `a41befb63e663890896ba67de48a51e65de49c4b281229a37b0e271a094e1943`;
- result artifact SHA256:
  `ff48d2285f3f34a87c608780bb98fe344606eab511c6b3f4ba834c68f0e01837`;
- decision SHA256:
  `e2f76cae70e8018888e9ded4bd0b162cb05957ff5deb2917a6eb4a4e994d3803`;
- calibration identity SHA256:
  `138ef55b8c1f9d48cc92e98fb17c7976d1bf86a6df37f851e5284668ce92ff1b`;
- manifest identity SHA256:
  `00c375a82a9a5b98a7011af26e603ab53c9779fae83199f81100154382c8be50`.

All `846` predeclared rows completed and were admitted: `540` fit, `72` model-selection, and `234`
final-holdout rows; `423` target and `423` control rows; `282` rows for each repeat index. There were
zero execution failures and all `846` row identities were unique. The execution identity binds:

- `guest-launcher` SHA256
  `59cb045b25a72e4a59ea5894085ef35952f0156eba0b9da051c44ede9612bef6`;
- production `sp1_shasta_proposal.elf` SHA256
  `ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7`;
- production verification-key SHA256
  `bd71c1f1cbd72387aa057ae6e936afd6dcb0bc170c6a91193c6d1a0b47507102`;
- trace reconstruction source SHA256
  `e35e6860d2d1fec6f9e76089fa4a14be218df3bf257d4ddbafbd378ce7173077`;
- production parity identity SHA256
  `39c7ba3fe67f8046d74e36090afdd0f06d9769b940a92da419f82eafb4d66319`.

The parity row remained the frozen `address_canonical / fit / count=1 / target / repeat=0` contract,
with row identity `a1116979a652d65df11dc489698e4359e2f1198205f52d3ffc4f0294373882ca`.
Standard and gas-estimator execution agreed on public output, gas, instruction count, syscall count,
guest-input identity, trace identity, and exit code.

The six family reports are retained as exact diagnostics, but every family was rejected:

| Opcode | Diagnostic function | Exact fitted coefficients | Decisive gate outcome |
| --- | --- | --- | --- |
| `0x30 ADDRESS` | `address_constant` | `22.39736880136499416652483455683297534744654566024311629262663198974926276313783` | Rejected: fit R2/residual, relative coefficient stderr, insufficient signal, and control contamination (`4780.0856756634... > 2860`). |
| `0x33 CALLER` | `caller_constant` | `12.423761763241827011099614615484001740408422493087691072685283016142224639970674` | Rejected: fit, signal/control, count/extrapolation, final-holdout, and sibling-consistency gates. |
| `0x34 CALLVALUE` | `callvalue_classes` | zero `-53.38855495230069498303528274521981057630712002890644382467542079617449090255132`; nonzero `-54.318173720629140730835869255483740195075448474654244411185684725793259230997067` | No selected candidate; negative coefficients, fit/stderr, insufficient signal, control contamination, and sibling consistency rejected it. Signed coefficients are diagnostic only. |
| `0x35 CALLDATALOAD` | `calldataload_access_classes` | zero `128.55900732966437957253284718155413237517853126047376458878825156569666660486804`; partial `212.29947653787552326754750993815237284438674240416877925154484980616587481601173`; full `161.10299559945910098015748061263917636344832598188138922221933660968493639958944` | Rejected: fit residual, insufficient signal, count/extrapolation, final-holdout, and sibling-consistency gates. |
| `0x36 CALLDATASIZE` | `calldatasize_length` and `calldatasize_boundary` | length: intercept `-50.403392753324387700347107844807401795283238723085316596866502102681394527822523`, length `-0.22099127263812179560263029537246659197418737639951961363357766974894583309402594`; boundary: intercept `-50.110084766572444738656006107878657106121391778662064548038079642704305174301075`, words `-4.051686217008797653958944281524926686217008797653958944281524926686217008797654`, partial `-2.9925464320625610948191593352883675464320625610948191593352883675464320625610948` | Both candidates were evaluated and rejected; no candidate was selected. Negative coefficients, fit/stderr, insufficient signal, control contamination, and sibling consistency failed. Signed coefficients are diagnostic only. |
| `0x42 TIMESTAMP` | `timestamp_nonzero` | `-63.461868735291897329076338463694883890090111231252484880393895869488273893753666` | No selected candidate; negative coefficient, fit/stderr, insufficient signal, control contamination, and sibling consistency rejected it. Signed coefficient is diagnostic only. |

The sealed outcome is therefore `result_status=rejected`, with zero promoted families,
`candidate_eligible=false`, `discovery_numeric_parameters_promoted=false`, and
`production_registry_modified=false`. Same-checkout replay, clean detached-checkout replay at
`dda214e385ee9608ad88ae18eefe3b7111224f45`, independent adversarial review, and independent
behavioral verification all passed.

An earlier `09e4c7e92a8758f4bce4424e` directory exposed a report-generation bug that omitted determinable
diagnostics for coefficient-less rejected candidates. It is uncommitted diagnostic evidence, is not
a valid result, and must not be used or sealed as a successor source. Result `a41bef...` contains the
corrected exact reports without changing any sampled row.

No proposal or final validation corpus was opened. No production estimator, registry, operation
coverage table, runtime configuration, schedule, block limit, Boundless configuration, or other
production parameter changed.

## Completed Milestone: Declared Context Approximation

The strict result above remains rejected and immutable. A separate review-only approximation was
derived from its same-production-guest target/control rows and sealed as
`experiments/opcode-gas/derivations/1d2758bcb7aa3ae7f09d14eb`:

- implementation revision: `60f6c2146cda122a5fd9a269173c7ba39c31a5c1`;
- approximation identity SHA256:
  `1d2758bcb7aa3ae7f09d14eb8d539250747992c42d6ae95d204f4cc5efec06cb`;
- artifact SHA256:
  `357a6c47bad8e6def3280be77a300e113e350fa673e08c39af13034861aee3d8`;
- canonical artifact-file SHA256:
  `346c2ac112357872f4a981737469c39639d7cc845ae97b0cc8bb77ddfc8a5b58`;
- strict source identity SHA256:
  `a41befb63e663890896ba67de48a51e65de49c4b281229a37b0e271a094e1943`;
- inventory: nine typed classes across the six context opcode families and `327` exact-repeat,
  nonzero controlled observations;
- policy: `declared_approximation`; no strict family was relabeled or promoted.

For every scenario, target and control are paired within the same repeat at count zero and at each
nonzero count. The fit uses
`D_s(n) = (P_target(n) - P_control(n)) - (P_target(0) - P_control(0))`, then adds exactly once the
canonical V5 cost of the lane-exclusive replacement instruction. Removing the target event and the
replacement event must leave identical raw-gas and event-count ledgers. Fixed context operations
use one `PUSH0`; `CALLDATALOAD` uses only its second, lane-exclusive `SWAP1`, so shared cleanup is not
charged twice. Each typed class selects the maximum finite nonnegative scenario cost.

Sealed per-event costs in SP1 normalized `proverGas` are:

| Class | Declared cost |
| --- | ---: |
| `address` | `97.649129037858121176747031351368990855052366915359737824471331436422526620808534` |
| `caller` | `88.011629037858121176747031351368990855052366915359737824471331436422526620808534` |
| `callvalue:zero` | `22.730379037858121176747031351368990855052366915359737824471331436422526620808534` |
| `callvalue:nonzero` | `21.780379037858121176747031351368990855052366915359737824471331436422526620808534` |
| `calldataload:zero` | `162.2406529527636473664319181838285678034906249065125651137631293968871533473679784164222873900293255` |
| `calldataload:partial` | `239.4840547122944391552882231691658112052501556983014214187484666402889128781597672727272727272727273` |
| `calldataload:full` | `202.9825884366346151083674020547963097389744958742545005976340971388226372183357203519061583577712610` |
| `calldatasize` | `28.96432331938304786296404014902294979933389184204595483326898539536680814573522021700879765395894428` |
| `timestamp` | `21.292879037858121176747031351368990855052366915359737824471331436422526620808534` |

The largest controlled whole-guest materiality is
`0.00001431259513559476800353930390251359861648052115561643635389931983088029171898040545852075899095550709`
(`0.0014312595%`), at `calldataload_partial_31_offset_30`, count `64`. This is training/diagnostic
evidence, not the acceptance result. Acceptance moves to a newly frozen controlled-block split.

Focused approximation and CLI tests passed, as did Python compilation, diff checks, create-only
collision, tamper rejection, and exact replay. Independent review fixed repeat collapsing and
publication durability before source commit. Independent arithmetic verification recomputed
ADDRESS/PUSH0 and CALLDATALOAD/SWAP1 without calling the builder. Artifact review then found and
closed a sealed-purpose overwrite before this final identity was generated. The artifact is mode
`0444` and its CLI verifier replays successfully.

No proposal or final-validation row was opened. No production table, registry, schedule, runtime
configuration, block limit, Boundless configuration, or deployed component changed.

## Reviewed Pre-Fixture Checkpoint: Fully Evaluable Review-Only Candidate

The declared context approximation is now integrated into a schema-4 composite estimator and sealed
at `experiments/opcode-gas/estimators/0bae279f01efa6c86ddd482c`:

- implementation revision:
  `5ff4ed2cfbb0cfc15bc96a931727ae026436d868`;
- full candidate identity SHA256:
  `0bae279f01efa6c86ddd482cebb344a335f7d70ee2711a0769fe31a95fb3cc4e`;
- canonical estimator-file SHA256:
  `6d47ac014924173a733e9cc4ba269f4f1602730d35d38908044ab2ae9577a256`;
- guest-launcher SHA256:
  `c839578e145cd6010bf6165857fe359dd872e50b30ae856a28cc1f32b5b160d3`;
- production SP1 proposal ELF SHA256:
  `ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7`;
- production SP1 proposal VK SHA256:
  `bd71c1f1cbd72387aa057ae6e936afd6dcb0bc170c6a91193c6d1a0b47507102`;
- trace reconstruction SHA256:
  `e35e6860d2d1fec6f9e76089fa4a14be218df3bf257d4ddbafbd378ce7173077`;
- trace inspector SHA256:
  `94b1da5fb5aad50da840978dd76ae0674161b242bd485d831ef6cbfaf3703f51`.

The estimator preserves the corrected Osaka V5 registry byte-for-byte and adds a separate derived
coverage overlay for exactly `ADDRESS`, `CALLER`, `CALLVALUE`, `CALLDATALOAD`, `CALLDATASIZE`, and
`TIMESTAMP`. The overlay consumes the nine sealed typed class costs once per executed event and does
not multiply them by raw EVM gas. Missing or incompatible typed input remains a coverage gap; no
current-schedule fallback exists. The six derived rows bind the current trace inspector rather than
the predecessor V5 source hash.

The candidate also binds the corrected core, V5 coverage ledger, corrected higher-layer package,
stateful storage result, declared context approximation, estimator source, implementation revision,
trace schema, launcher, ELF, and VK. It remains `review_only=true`, `production_write=false`, and
`proposal_validation_opened=false`.

Independent source review found and closed two blockers before the source commit: incomplete
coverage had required a fabricated prediction, and the derived context rows retained stale trace
provenance. In the final implementation, incomplete coverage accepts only a null candidate
prediction, produces no candidate aggregate metrics, and fails every candidate gate; a numeric
fallback is rejected. The current inspector path, selector, and SHA are validated on all six derived
rows. Final source review and behavioral testing passed, including schema-2/3 replay, all nine typed
classes, raw-gas independence, exact Decimal comparison gates, create-only publication, replay,
collision, and tamper rejection.

The sealed candidate then passed independent artifact review and behavioral verification. Both
recomputed its content address and all bound source/execution hashes, replayed it through the CLI,
verified V5 registry equality, the six derived overlay rows, and the nine class costs, and confirmed
that no block manifest, block row, Unzen scalar, comparison result, proposal result, or validation
result had been created.

This candidate was correctly sealed and reviewed before any block result opened. Subsequent source
work found that the final Task 4 matrix requires additional bounded memory, storage, mixed, and
transaction-count fixture support in the guest launcher. Therefore `0bae...` is retained unchanged
as the reviewed pre-fixture checkpoint, but it is not the candidate that will bind the final frozen
manifest. After the structured fixture source passes native trace/parity review, the launcher must
be rebuilt and a create-only successor candidate must bind that launcher and source. No calibration
or validation observation has been opened while making this ordering correction.

No production table, registry, schedule, runtime configuration, block limit, Boundless
configuration, or deployed component changed.

## Reviewed Pre-Compact Checkpoint: Structured-Fixture Successor Candidate

The structured block fixtures, exact native replay gate, and schema-5 successor are now complete.
The reviewed successor is sealed at
`experiments/opcode-gas/estimators/72a5843c0c9031dd48014608`:

- implementation revision:
  `ec2ad0c0a55182cfa1e0b3c32f617a74ede93d86`;
- full candidate identity SHA256:
  `72a5843c0c9031dd48014608d4694ad2f9d3888fcafe8db62cec4456618badfa`;
- canonical estimator-file SHA256:
  `4ba2407f72db83835f2c18c292fd82b7c118ef8db263cb7a56e947d37bfe605a`;
- rebuilt guest-launcher SHA256:
  `4836a23aa1e6a6c5471cde7633951a2a4bfcbbaa964993d6e66e7aee9f4b2899`.

Relative to `0bae...`, the successor preserves the registry, storage model, fixed costs and
statuses, coarse state/trie model, context approximation, ownership and coverage policies,
formula, and version identity. Of 168 execution-coverage keys, only `opcode:0x00` changes: the
schema-5 overlay accepts exactly a real `STOP` event with zero raw gas and assigns zero direct cost,
leaving any residual terminal work to the transaction-base term and the block-level acceptance
gate. The successor additionally binds the structured fixture source and block-manifest builder.

Independent artifact review recomputed the logical and file identities, matched all 31 bound
source and execution hashes, replayed the exact verifier, and confirmed the model-preservation
claims above. Source verification separately ran all 32 fixed native rows through
prepare/build/seal/verify, for 128 audited launcher invocations, with complete candidate coverage,
zero gaps, positive finalized Unzen zkGas, and no SP1 execution. Tampered identities, sources,
bundles, inventory, symlinks, collisions, and launcher TOCTOU all failed closed.

No controlled-block manifest, observed proverGas row, Unzen scalar, comparison result, proposal
result, or validation result was created while sealing this candidate.

The first real prepare exposed that embedding every deterministic `candidate_trace` would make the
32-row manifest about 147 MB. No data had opened, so the builder was changed before manifest seal:
schema 2 stores the complete compact identity evidence, canonical full-bundle and trace hashes, and
the candidate prediction, while public seal and verify still regenerate every full trace with the
bound launcher. The resulting manifest is about 140 KB. Because that builder source is candidate
bound, `72a584...` remains immutable but is no longer eligible for the final manifest.

## Reviewed Pre-Portability Checkpoint: Compact-Manifest Candidate

The final reviewed successor is sealed at
`experiments/opcode-gas/estimators/464c3d49e3ddd17090d6b8fe`:

- implementation revision:
  `cb0a536959dacbb89173e9b46103260c76dac21c`;
- full candidate identity SHA256:
  `464c3d49e3ddd17090d6b8febe8a35b128da6dfaa42efebda75c85b600ab64b8`;
- canonical estimator-file SHA256:
  `88f273b6317423bb7ec3a495615657afb60948fb9b3d9ecb03b09f9ece0a90bb`;
- rebuilt guest-launcher SHA256:
  `4836a23aa1e6a6c5471cde7633951a2a4bfcbbaa964993d6e66e7aee9f4b2899`.

Relative to `72a584...`, only the implementation revision, compact builder source hash,
revision-bound higher-layer projection identity, and total artifact identity change. The registry,
storage model, fixed/coarse models, policies, formula, context and terminal overlays, trace schema,
and all 168 coverage rows are identical. Independent review recomputed both identities, matched all
31 bound source/execution hashes, and passed exact replay. The header remains review-only with
`production_write=false` and `proposal_validation_opened=false`.

## Reviewed Pre-Portability Checkpoint: Frozen Controlled-Block Manifest

The source-only block manifest is sealed at
`experiments/opcode-gas/manifests/sp1-block-comparison-v1/bd0652fb4a5a5999d558410c`:

- full manifest identity SHA256:
  `bd0652fb4a5a5999d558410c5aac9a5fbc78b3d1c50efdd0bf97c2823f4efa6f`;
- canonical manifest-file SHA256:
  `d02eda0c5cab45c250febe6f3bdafbffe24f8b0bf3644447ac38e560eedeb11b`;
- fixed matrix SHA256:
  `132e1b6f6210c0f1f07a8dd1e9af1cda6dfe2e8ee623f74711eca32625abf44e`;
- bound candidate identity SHA256:
  `464c3d49e3ddd17090d6b8febe8a35b128da6dfaa42efebda75c85b600ab64b8`.

The 140,705-byte schema-2 artifact freezes 12 calibration rows and 20 untouched validation rows.
Both partitions cover all nine workload categories; together they cover all nine declared context
classes and the storage set, clear, reset, and restore-original branches. Every row has complete
candidate coverage, a deterministic prediction, unique bundle and trace hashes, and mutually
disjoint workload, row, and backend-input identities. The compact artifact contains neither the
full `candidate_trace` nor a `frozen_bundle`; exact seal and verify regenerate those bytes with the
bound launcher and require full bundle, compact evidence, trace, identity, coverage, and prediction
equality.

Independent artifact review recomputed all content, matrix, candidate, and seven source hashes.
Independent behavioral verification observed exactly 32 native launcher executions and zero SP1
executions, then repeated a separate 32-row replay. Both passed. Compact/full hash, prediction,
identity, source, launcher TOCTOU, and self-consistent swapped-trace tampering all failed closed.
The manifest is canonical, mode `0444`, and contains no observed proverGas, scalar, normalization,
comparison, proposal, or verdict field. The validation partition remains unopened.

The local sealer correctly created `bd065.../manifest.json` as mode `0444`, but Git cannot preserve
read-only mode bits: a fresh checkout materializes every non-executable tracked file as `0644`.
The verifier originally accepted only `0444`, so `bd065...` could not be replayed from a fresh
clone. Before data-open, the verifier was narrowed to the two portable canonical modes `0444` and
`0644`; it still rejects other modes, symlinks, extra inventory, source drift, and all replay
mismatches. This builder change makes `464c3d...` and `bd065...` immutable pre-portability
checkpoints rather than the pair eligible for calibration.

## Completed Milestone: Portable Final Candidate And Manifest

The final candidate is sealed at
`experiments/opcode-gas/estimators/c3f24358f8a2b702658a6496`:

- implementation revision:
  `cac07a74b4b6d8d72806b99c11761ed6dbdc3f51`;
- full candidate identity SHA256:
  `c3f24358f8a2b702658a6496431e06d589c54fadaeb9dea34b5aa3805db115aa`;
- canonical estimator-file SHA256:
  `19b548ef041f7e9a99ed217b11ec5a40377f276b5ba09832777f6f1a018b9907`.

The final manifest is sealed at
`experiments/opcode-gas/manifests/sp1-block-comparison-v1/658fc415d93188e7ee1e0584`:

- full manifest identity SHA256:
  `658fc415d93188e7ee1e058449558c3041b14bae5e23f0f5f7272763a38efc85`;
- canonical manifest-file SHA256:
  `e0658953033ad7067de90b6f11c38f3f6e4d4737b0d6245c74c7b3e0a1c93f75`;
- fixed matrix SHA256:
  `132e1b6f6210c0f1f07a8dd1e9af1cda6dfe2e8ee623f74711eca32625abf44e`;
- bound candidate identity SHA256:
  `c3f24358f8a2b702658a6496431e06d589c54fadaeb9dea34b5aa3805db115aa`.

The candidate differs from `464c3d...` only in revision-bound provenance. The manifest differs from
`bd065...` only in candidate/builder provenance and the repeated candidate identity; all workloads,
compact evidence, bundle and trace hashes, and predictions are identical. Independent review and
behavioral verification both passed with the final manifest in fresh-checkout mode `0644`. The
formal CLI executed exactly 32 native launcher calls and zero SP1 calls, and a separate 32-row
replay reproduced every full bundle, compact identity, trace, ID, coverage result, and prediction.
The validation partition remains unopened.

## Completed Milestone: Sealed Block Calibration And Unzen Normalization

The post-freeze runner landed at implementation revision
`daea85db579854689445f18ee6ea4c50a59047b1`. It pins the sole eligible
`c3f243...` / `658fc...` pair, verifies the full manifest before data open, executes only the 12
calibration rows with the production SP1 proposal ELF and gas estimator, and binds the exact Python
implementation sources, launcher, ELF, VK, trace, fixture, and builder. Each row has a 600-second
hard timeout and an atomic create-only checkpoint. Independent source review and behavioral testing
passed before the first real SP1 execution.

The ignored resumable run
`experiments/opcode-gas/runs/658fc415d93188e7ee1e0584-calibration` completed exactly 12 input and
12 result rows at that revision. All row IDs are members of the frozen calibration partition; no
validation row was executed or copied into the run.

The exact calibration result is sealed at
`experiments/opcode-gas/calibrations/sp1-block-comparison-v1/c785d807bf97dbf8c46eb66a`:

- full calibration identity SHA256:
  `c785d807bf97dbf8c46eb66aed27a564ba6b2826b442160a60d2bebfe121651f`;
- canonical calibration-file SHA256:
  `b0139d961fa990996ce5951ea3527bc4b7e6ed7f5ca35a482d3b8fa7f275ff57`;
- file mode and size: `0444`, 69,644 bytes;
- row count: 12 calibration rows and zero validation rows.

The separate Unzen normalization is sealed before validation at
`experiments/opcode-gas/normalizations/sp1-block-comparison-v1/9d97015d76b8c5d66f28f8d2`:

- full normalization identity SHA256:
  `9d97015d76b8c5d66f28f8d220138f91aee740adcd083d62f5938219b5a5470e`;
- canonical normalization-file SHA256:
  `3856ea6deca594d80c8d68d698cfce1faac037110f02049a6db2c0c6a47b72c7`;
- file mode and size: `0444`, 4,270 bytes;
- `kappa_unzen`:
  `84.3750814595324998189721280545860481398170409774561269072240732568704027429022289800884237437716668`.

The normalization stores the exact median formula and the predeclared acceptance thresholds from
the comparator's single source of truth. Public exact verification replayed the bound native
manifest and both sealed artifacts successfully.

Calibration diagnostics are descriptive only and did not change a coefficient or threshold. On
these 12 fit rows the frozen candidate has `0.14055%` MAPE, `0.52525%` maximum APE, `-0.14055%`
mean SPE, and `0.52525%` maximum UPE. All candidate errors are small underpredictions; the two mixed
rows produce the largest residuals. The scalar-normalized Unzen baseline has `31.09887%` MAPE,
`343.70590%` maximum APE, `30.68136%` mean SPE, and `0.54106%` maximum UPE on the same calibration
rows. These fit-partition numbers are not the acceptance verdict and must not be used to repair the
candidate.

## Completed Milestone: Untouched Block Validation

The validation-only runner landed and was pushed at source revision
`9fb28d648e60ab4d652cd7dc170e59043d1f00e2` before the validation partition was opened. The runner
requires the exact `c3f243...` candidate, `658fc...` manifest, `c785...` calibration, and `9d970...`
normalization. It pure-read checks all seven bound source paths and hashes before native replay,
executes only the manifest's 20 validation rows, and uses create-only resumable checkpoints with a
600-second timeout per row. Independent source review and behavioral testing passed before the real
run; both review findings were fixed and rechecked.

The ignored run `experiments/opcode-gas/runs/658fc415d93188e7ee1e0584-validation` completed all 20
validation rows once. Its 20 input IDs and 20 report IDs exactly equal the frozen validation
partition, have zero overlap with the 12 calibration IDs, and bind source revision `9fb28d64...`.

The exact validation evidence is sealed at
`experiments/opcode-gas/validations/sp1-block-comparison-v1/d1e00d3424e2ed5b93c04231`:

- full validation identity SHA256:
  `d1e00d3424e2ed5b93c04231ae37bafbc0640c20ef10c55ae0e2fb8a73f20051`;
- canonical validation-file SHA256:
  `3e5875c5b6d6a2da832b26599a84b8839cb85c79e4dd332e9e672113d84b8b83`;
- file mode and size: `0444`, 120,680 bytes;
- row count: 20 validation rows and zero calibration rows.

The one predeclared comparison is sealed at
`experiments/opcode-gas/comparisons/sp1-block-comparison-v1/fc28fac748793362d7fe40f5`:

- full comparison identity SHA256:
  `fc28fac748793362d7fe40f5041957566627d26f18ec21a7cd54d52ccf172de2`;
- canonical comparison-file SHA256:
  `eb0e393750da30461e4ff27bcea03539e2db560f3a014ca5eacb553467e8bf7c`;
- file mode and size: `0444`, 25,169 bytes;
- result: `block_validated_against_unzen`; all eight explicit gate booleans are true.

On the untouched 20-row partition, the candidate MAPE is `0.17406%` versus `14.61521%` for the
scalar-normalized Unzen baseline. Candidate maximum APE is `0.51574%` versus `148.55504%`; mean SPE
is `-0.17406%` versus `14.28769%`; p95 UPE is `0.50534%` versus `0.54049%`; and maximum UPE is
`0.51574%` versus `0.54106%`. All candidate residuals are small underpredictions, but the worst is
only about `0.52%`, well inside the predeclared `10%` p95 and `20%` maximum underprediction limits.
No coefficient, scalar, threshold, or repair offset was changed after validation opened. Independent
public verification completed in 93.40 seconds with exactly 32 native identity replays, zero SP1
executions, and an exact independent Decimal recomputation of every row, aggregate, and gate.

## Next Gate

Strict Task 7 is not executed. Result `a41bef...` remains ineligible for strict promotion and its six
family verdicts remain rejected. The separately sealed approximation above is not a reinterpretation
of those verdicts. The `0bae...`, `72a584...`, and `464c3d...` estimators remain review-only
historical checkpoints. The reviewed `c3f243...` candidate and `658fc...` manifest are the only pair
eligible for this experiment; neither is a production artifact. Their 12-row calibration,
`9d970...` normalization, 20-row validation, and comparative verdict are now sealed. SP1 diagnostics
remain explanatory sidecar data and never enter the online formula.

The decisive comparative block condition has passed without a result-time adjustment. This result
does not by itself promote the candidate to the production schedule or replace proposal-level
validation. The next experiment may use the sealed block result as input, but may not reinterpret the
20 validation rows as calibration data.

After this context family, close the remaining ordinary returndata/copy/LOG/EXTCODE keys before CALL
wrappers and direct precompiles. Reuse sealed controlled evidence where its execution semantics and
model inputs are sufficient; otherwise predeclare the smallest additional controlled campaign.

## Completed Milestone: Integration-Smoke Checkpoint

The two preflight-validated smoke joins completed on 2026-10-02 with the sealed schema-5 terminal
candidate `experiments/opcode-gas/estimators/36614e3963955da7cbefb944/estimator.json`
(`36614e3963955da7cbefb94468352ef2f62e53921a18489bafa71536bb4e208c`). It binds implementation
revision `1f5d05777172001b96122e4675a7abc0eb74fe60`, launcher SHA256
`46bed3a3a31a165199f29e36e8ac64b07c3bc60b70ee4886afb850973d41e4de`, proposal ELF SHA256
`ba5ab7026643730c4416550a069fd0776f0e11ae09155ec5fce413acbe5e59c7`, and proposal VK SHA256
`bd71c1f1cbd72387aa057ae6e936afd6dcb0bc170c6a91193c6d1a0b47507102`. Its model content is the
same as `c3f243...`; only revision/provenance/source hashes, projection identity, launcher hash, and
artifact identity changed.

- Hoodi `80907` completed its trace and parity join for 378 blocks. Its GuestInput JSON SHA256 is
  `f04d297060e42c74540233c84bc44514bc2a5eb0dbc13111d85bb0522bc93d8f`; the guest report's bincode
  hash is `0x64083abb0a43a73654b75ce4d8856083fdbc98657856f0bc9886a812ed07d7a2`. The input has 2,257
  transactions: 378 Anchor plus 1,879 non-Anchor native transfers. All 4,632,511 operations are
  Anchor/block-owned, so the candidate operation-coverage denominator is zero. Actual `proverGas`
  is `2,318,475,174`; predicted `proverGas` is
  `1,437,555,380.9303051854641007206113351763106672341486303454488711019759931168028533`; APE is
  `0.37995653477275267754749712035892812354493331566351706578641540287806717019310499`; wall time
  is 152,194 ms. The non-gating diagnostic totals are 2,102,180,920 instructions and 3,017,404
  syscalls. The plumbing and join passed, but model validation failed. The residual must not yet be
  assigned to state/witness work: the current ownership policy excludes every Anchor operation from
  operation pricing and absorbs it into one fixed `block_base`. This candidate must not be retuned
  from the smoke.
- Mainnet `39339` completed its trace and parity join for 192 blocks. Its GuestInput JSON SHA256 is
  `c92dbb3c9f5c5142bc145dfa322162c2a100007641f20f81f705e8d6e273dc95`; the guest report's bincode
  hash is `0x7e79d122ff315a200b9e5362b2614d961e52cd09ead8935ff875169d3fcb8c0a`. The input has 198
  transactions: 192 Anchor plus 6 non-Anchor contracts. Actual `proverGas` is `2,123,206,252` and
  the modeled subtotal is
  `642,506,644.86162051361197726710081468582625339056952960491739497663687256083429844`; it is not
  a prediction and has no APE because the run has 270 gaps: 149 unsupported opcodes, 76 spawn
  wrappers, and 45 precompiles. Operation coverage is 35,359/35,629 (`0.9924219035`), raw-gas
  coverage is 427,564/968,870 (`0.4413017226`), typed-feature coverage is 1, and precompile/spawn
  coverage is 0; wall time is 114,237 ms. The non-gating diagnostic totals are 2,261,698,066
  instructions and 1,074,671 syscalls. The principal gap keys are `0x37` (50), `0x3d` (27), `0xf3`
  (25), wrapper `0xfa` (62), and precompiles `01`/`02`/`06`/`07`/`08` (45 total). The subtotal must
  not be treated as a prediction.

A read-only counterfactual then routed Anchor operations through the existing operation models while
leaving the sealed estimator and traces unchanged. This is diagnostic only: the current `block_base`
was calibrated with Anchor work absorbed, so the counterfactual subtotal would double count an
unknown part of that work and is not a prediction or APE.

- Hoodi could price 4,531,198 of 4,632,511 Anchor operations. Those measured operations contributed
  `442,549,195.55475755606099353352076871289646630467862313559203551012186682522449474`
  proverGas. After removing the synthetic transaction-base charge used only to exercise the strict
  evaluator, the diagnostic subtotal was
  `1,880,104,576.485062741525094254`. Its 101,313 gaps were dominated by 97,146 `BLOCKHASH`
  (`0x40`) executions; measured operation-count coverage was `0.9781300033` and raw-gas coverage was
  `0.9071043264`.
- Mainnet could price 2,337,161 of 2,388,896 total operations after the same routing. The resulting
  measured-operation contribution was
  `228,591,481.82078628227567617629357122107379116773921893094205616600009756413095467`
  proverGas, of which
  `224,822,414.8296976280594355414210140270273718720364299731418537421530608679864806128`
  was newly visible Anchor work relative to the formal smoke subtotal. The diagnostic subtotal was
  `867,329,059.6913181416714128085` after removing the synthetic transaction-base charge. Its 51,735
  gaps were dominated by 49,344 `BLOCKHASH` executions, alongside wrapper, precompile, and
  returndata/copy/LOG gaps.

This closes the interpretation question, not the model: Anchor is an opcode workload, not a fixed
opaque block charge. The next bounded experiment must first price Anchor operations through the
ordinary operation layer and calibrate `BLOCKHASH` plus the remaining material operation families.
Only after refitting `block_base` without absorbed Anchor opcode cost may a remaining residual open a
new state/witness or proposal-level model. This preserves the opcode -> state/trie -> transaction ->
block -> proposal boundary instead of explaining a lower-layer coverage hole with a higher-layer
offset.

The final 60-row corpus remains unopened. Neither smoke result changes a coefficient, production
configuration, or the candidate's eligibility. Instruction count remains only a cycle proxy: these
diagnostics do not fit or justify a temporary bridge.

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
