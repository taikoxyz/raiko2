# Raiko2 Etna Integration Plan and Progress

> **For agentic workers:** Use superpowers:subagent-driven-development or
> superpowers:executing-plans for implementation. Track work by the stable P00–P08
> IDs below. Check a task only after its acceptance evidence is recorded.

**Goal:** Prove supplied Etna ranges with shared consensus primitives and Raiko2's
existing execution, witness and prover infrastructure.

**Architecture:** Raiko2 composes narrow verification calls around the existing
EVM replay and proof backends. Producer policy and BFT networking stay outside the
guest. Preserve CachedTrie, witness pooling, execution DB and Unzen behavior.

**Tech stack:** Rust, etna-core, existing Alloy/prost/Comet codecs, alethia-reth,
RISC0 3.0.5 and SP1 6.8.1 at the recorded feasibility checkpoint.

**Spec:** [Shared proving design](../specs/2026-11-17-etna-shared-proving-design.md),
including its current planning-baseline amendment.

## Active L2-managed QC integration (2026-11-27)

The user explicitly resumed QC implementation against [design #22425 at
`4bb5cff1`](https://github.com/taikoxyz/taiko-mono/tree/4bb5cff1e9e4f96506a8ee971e430014188619ac/packages/protocol/docs/etna),
client #22280 at `87f95412` and contracts #22286 at `9b8d5b4c`. This supersedes
QC's earlier blanket hold and the old registry/epoch/21-word implementation
target. These are development pins, not merged/frozen/released deployment claims.
Preserve the existing EVM, sparse trie, witnesses, queues and providers. FI phase
two and actual SDK/ELF/image/deployment/chain acceptance remain separately gated.

### Task 15: Current shared core and landing encoding

- [ ] Carry the existing exact raw-L1-header API onto current no_std etna-core
  without restoring registry/epoch APIs or reorganizing the client. Preserve
  the existing #22426 extraction history; no duplicate dependency PR or force
  update of the shared development branch. Inspect the complete minimal carrier.
- [ ] Adopt the immutable carrier; align current params/domain/member APIs and
  seven-field LandInput / 19-word (608-byte) statement and the current
  three-field `[block, anchor, forced]` envelope. Reject the old four-field
  committee-witness form and preserve forced-witness positioning. Reuse authoritative
  config hashing. Reject old registry/active-snapshot inputs, preserve every
  ordered cut and full-width public field, and retire the unused superseded
  certified-envelope gate rather than add an always-failing compatibility path.
- [ ] Independently check encoding against current contract ABI and exercise
  codec/range regressions through exact-source native CI.

### Task 16: Authenticated state to committee, bitmap and certificates

- [ ] Reuse core committee::decide, staking storage layout, fallback keys and
  miss bitmap functions. Derive all inputs from authenticated L2 parent/post-state
  and SSZ-bound L1 anchor state; verify origin/bootstrap and current/next hashes.
  No supplied schedule, detached state root or host approval authorizes a set.
  This is input verification only: no vote/QC collection, RPC acquisition,
  synchronization, signing, validator updates or producer policy is added.
- [ ] Read required snapshot/key-to-seat/Inbox words using the existing sparse
  witness engine, with one hydration per batch. Missing material fails; proven
  absence remains distinguishable. Do not duplicate account/trie verification
  or reimplement staking penalties that the existing EVM executes.
- [ ] Authenticate LastCommit bytes and derive every bitmap without adding
  signature checks the spec omits for that purpose. Verify strict weighted QCs
  at the terminal and before actual key/power changes, with exact coverage,
  stock canonical bytes, generation domain and parent/range boundary checks.
- [ ] Compose these checks with existing strict DA/SSZ/EVM verification in an
  actual native consumer. Keep remaining profile/time/rules/FI and L1 landing
  authority obligations explicit; no partial-result signing route.

### Task 17: Independent integration verification and publication

- [ ] Independently review the complete source/pins/ABI and exercise genuine
  EVM/MPT plus signed-QC success and mutation cases, including parent bootstrap,
  fallback, source-equivalent sets, absent witness material and forged roots.
  Fix and have the raiser recheck all material findings.
- [ ] Run focused native CI at the exact source and effective merge, inspect
  actual tests and steps, update this plan and draft #305, and report remaining
  gates. Use the existing shared target only; at the current disk reserve do
  not start local Cargo/SDK/ELF/image builds. Record unexecuted RED separately.

The approved explicit lander-selected Q and native uniform SSZ components remain
in use. Their normative history/recovery reconciliation is still open; the QC
migration neither changes reference policy nor declares onchain acceptance.

## Native input checkpoint (2026-11-27)

Continue the SGX-first plan through owned input plumbing. The current native
range relation remains a component: authenticated context/profile/committee and
QC obligations are not replaced by a host assertion or a serialization adapter.
Use the existing execution witnesses, shared nodes, KZG, SSZ and stateless replay.
No new prover architecture, SGX endpoint or signing path is introduced by this
batch. The encoded-input byte limit is an outer caller obligation; serde is not
a pre-allocation resource limiter.

Native checks at source `95c93ad8` passed in
[CI](https://github.com/taikoxyz/raiko2/actions/runs/45956473290), effective merge
`82366763` into base `3f4ec387`: workspace formatting, full native Clippy
(including Etna all-target tests and engine core), foundation, pipeline and runtime. Independent verification read
the actual checkout/effective merge and all 15 new test names, each successful.
The foundation logs record 559 passing test invocations, zero failures and one
ignored pre-existing doctest across 33 summaries; repeated MPT tests are included.
Five new cases were independently authored. Coverage includes real EVM/MPT
range roundtrips, list-count mismatches, malformed/interior transactions,
Q/raw-header and authority-field mutations, caller budgets, and public-mainnet
sparse SSZ roundtrip/helper mutations. Consensus/context in the EVM fixtures
remains synthetic. No fetched devnet, enclave signature or onchain acceptance
is established by these native tests.

Independent source review passed on the exact source. Full native tests ran
in CI; local RED/GREEN compilation was not run at the shared-disk reserve.
Local workspace fmt/diff checks and offline locked metadata passed. All package
versions, sources and checksums remain unchanged; only already-resolved serde
edges were added to the locks. CI lint corrections changed documentation,
equivalent test literals, a unit-statement semicolon and line wrapping; no
runtime or verification policy changed.

Tasks 13–14 are complete as a native input component. SGX-01..04 remain open at
their stated boundaries: no production provider/pipeline route, complete
consensus/context composition, actual enclave signature or landing is claimed.
RISC0/SP1 artifact checks are outside this SGX-first batch, not waived merge gates.
No local Cargo build, new target, image, deployment, registration or chain write
was performed. This draft stays unmerged.

### Task 13: Owned native input and existing-verifier adapter

- [x] Add owned serializable range, statement and L1-history witness inputs in
  `protocol-etna`, preserving original bytes, explicit Q, complete range/cut
  lists and every SSZ helper/length field. Reject unknown JSON fields. Reuse the
  existing statement and leaf types; keep deployment context, resource limits
  and EVM configuration outside the serialized request.
- [x] Borrow those inputs into the existing native verifier through a narrow
  `guest-common` adapter. Decode canonical original transaction bytes, reuse
  existing per-block execution witnesses and shared state nodes, and return a
  digest only after unchanged full native replay succeeds. Keep the same
  context, consensus and authority obligations as the existing entrypoint.
- [x] Add deserialized real-EVM range success and mutation regressions, exact
  statement/history roundtrips and caller-owned budget/authority controls.
  Record RED/GREEN execution separately from source-only checks. No local
  Cargo/SDK/ELF/image build while the shared disk is at its reserve.

### Task 14: Independent verification and draft publication

- [x] Independently review original requirements, complete source diff and
  allocation/borrowing/failure paths; independently exercise the serialized
  input through real native tests. Recheck any material fixes.
- [x] Run formatting, exact dependency/lock checks and focused native CI; read
  actual checkout SHA, test names and outcomes. Update this plan and draft #305
  with observed evidence, keeping SGX-01..04 and existing merge gates distinct.

The generic SGX wrapper was rechecked against live protocol source
[`87f95412`](https://github.com/taikoxyz/taiko-mono/tree/87f954124e59ce8008c20ef186531bfb1609e222/packages/protocol/contracts/layer1/verifiers):
`LibPublicInput` hashes five ABI words (VERIFY_PROOF, L2 chain ID, SGX
subverifier address, statement digest, instance address); `SgxVerifier` accepts
89 bytes (uint32 instance ID, address, recoverable signature). This confirms
reuse potential, not an Etna Inbox deployment or complete signing relation.
This historical checkpoint used the previous 21-word statement and per-height
cuts, with no ValState settlement fields. The active L2-staking migration above
supersedes that layout with 19 words. Concrete verifier/Inbox identity,
complete authenticated context and QC relation must precede signing and landing.

## Native execution source checkpoint (2026-11-27)

B04-1/2/3 now have an implemented native entrypoint:
`verify_blob_range_statement_execution`. It composes existing DA/range/SSZ
checks with the existing strict supplied-block witness/EVM/post-state verifier.
It decodes the exact committed transactions afresh, recovers actual signers,
normalizes full witness headers, and binds every witness parent to the incoming
parent or preceding verified block. No candidate filtering, new executor or
trie is introduced. Projection-only entrypoints remain explicitly partial.

The source adopts current Etna 29-byte extraData and automatic staking calls
through immutable development carrier `9ecf843d` ([Alethia #272](https://github.com/taikoxyz/alethia-reth/pull/272)),
based on #248 `b4c4bb97`. The carrier restores only the old Unzen library helper,
its genesis constant and dependency edge; existing Unzen behavior is retained.
Normal host/MSRV, CI and image build declarations align to Rust 1.99. The
legacy BlobCoder EIPs edge remains separate from modern execution traits.

Genuine EVM/MPT regression tests cover empty/full/two-block ranges, ordinary
first reverts/OOG, system-call state changes, invalid interior transactions,
poisoned caches and forged parent/body/receipt/post-state commitments. Fixtures
use synthetic consensus/L1 context; no fetched devnet run is claimed.

Local exact old-decoder RED reproduced both shape defects (29 rejected/13
accepted). The full Rust 1.99 RED run hit a 512 MiB growth guard before product
compilation; no EVM test pass is claimed from that attempt. CI at source
`c05e20b6` (effective merge `295b8d9` into `3f4ec387`) then passed foundation,
pipeline and runtime. The foundation log records 544 passes over 32 summaries,
zero failures and one ignored pre-existing doctest; this includes repeated MPT
metrics tests. All 20 new execution test names passed: nine composer cases and
11 stateless cases, the latter also repeated under MPT metrics. Independent
behavioral verification read the actual checkout and test names/results.
Clippy found one misplaced import and Rust 1.99 warnings from async-trait's
injected must_use attributes; a separately reviewed behavior-preserving fix
moves the import and adds trait-scoped expectations for that generated lint.
Final source `ea27a735` passed fmt, Clippy, foundation, pipeline and runtime
CI; actual native test names/results were independently checked. RISC0 checks
are outside the SGX-first milestone. Actual metadata, lock identities, formatting
and independent source review passed. No local SDK/ELF/image or proof build,
SGX acceptance, merge, deployment or chain write was performed.

The new entrypoint has a native regression consumer, not a production SGX
provider/pipeline route. SGX-02..04 and authenticated context/QC/FI/profile,
L1 lookup and normative SSZ/recovery gates remain open. Mainnet/Hoodi Etna
activation rejects until upstream supplies their trusted staking addresses;
this batch's execution fixtures use the supported devnet.

## Active SGX execution batch (2026-11-27)

The user authorized direct implementation of B04-1/2/3. RISC0 recovery and
proof builds are outside this milestone; first compose native verification for
SGX. Existing QC management/FI-phase-two/context/contract gates remain separate.

### Task 11: Adopt current execution source and compose strict replay

- [x] Adopt immutable Alethia #248 source and compatible OP-reth/Alloy dependency
  identities. Preserve the deleted devnet Unzen API additively upstream rather
  than duplicating genesis policy in Raiko2. Development-branch pins are
  explicitly authorized for this draft; canonical-main adoption remains a
  merge-readiness gate. Use the source's required native Rust 1.99 toolchain
  without MSRV bypasses; keep SDK/ELF/image/proof builds out of this batch.
- [x] Add failing real-EVM regressions, then scope zero-root and privileged
  anchor/first-receipt assumptions by the trusted fork schedule. Keep legacy
  behavior and strict full-body execution, ordinary reverts/empty blocks and
  computed post-state checks. Never use candidate filtering for committed blocks.
- [x] Decode current 29-byte Etna extraData through the authoritative upstream
  type, retaining generation/anchor bindings. The upstream executor consumes
  the header's miss bitmap in its automatic block-start call. QC/miss-bitmap
  semantic authentication remains part of the held consensus design; no new
  policy or replacement executor is introduced here.
- [x] Connect the existing DA/range/SSZ composer to the existing witness-backed
  validator for every block, with exact committed transaction bytes, recovered
  signers, parent/header/body/state-root checks. Return the statement digest
  only after all replay succeeds. Keep projection-only APIs clearly partial.

### Task 12: Independent verification and draft publication

- [x] Independently inspect the complete dependency/source/carrier diff, fork
  behavior and failure paths. Independently exercise real EVM/MPT fixtures and
  parent/body/state mutations. Fix/recheck confirmed in-scope findings.
- [x] Validate normal metadata/locks, focused tests/lint and actual formatting;
  bind receipts to frozen product bytes. No new shared target or profile.
- [x] Update this plan/design and draft #305 with exact validation and remaining
  native-versus-SGX/runtime gates. No merge, deploy, registration or chain write.

## Native SSZ checkpoint (2026-11-27)

Tasks 9-10 now provide a native Q-bound verifier and Etna composer integration.
The public mainnet vectors include an actual 1,100,000-execution-block gap;
synthetic vectors cover history/fork/slot/budget boundaries. Frozen exact-source
native checks passed **213 tests**, and scoped all-target Clippy with
`-D warnings` passed. Independent source review and behavioral verification
completed; the two findings (unused anchor openings and uncharged legacy
state-root siblings) were fixed and rechecked, including a failing-before-fix
budget control. The existing upstream pin and Unzen policy remain unchanged.

These checks use a diagnostic facade with byte-identical product modules and
real cryptographic libraries; unrelated transport/error types are excluded.
They do not establish a full production guest-common build or SGX acceptance.
Production root/SP1/RISC0 default and guest bench metadata resolve offline;
lock package identities, checksums and SDK pins are unchanged. No SDK, ELF,
image or real proof build was run. The broader local library check remains
unstarted below its 6 GiB disk reserve; native checks used the narrower recorded
resource policy below. Existing CI artifact-download 403s remain separate.

Remaining activation gates: authenticated deployment/profile budgets, full
runtime/witness acquisition, L1's independent H(Q) lookup and contract wiring,
normative linear-recovery reconciliation, and existing source/provenance,
SGX/context, QC and FI holds. Q selection/retry stays outside Raiko2.

## Uniform SSZ native implementation batch (2026-11-27)

The user authorized implementation of the explicit-Q direction. Execute Tasks
9 and 10 in this existing worktree; no second project, target directory or SDK
build. Existing P00-P08 delivery and QC/FI gates remain distinct from this native
anchor-authentication component.

### Global Constraints for Tasks 9-10

- Lander chooses Q before requesting the Q-bound proof. Raiko2 does not choose,
  refresh, switch or retry Q. Preserve the existing statement/domain/DA fields.
- Preserve EVM, CachedTrie, witnesses, provider/pipeline, backend/ELF/image pins
  and Unzen policy. Replace only Etna's linear ancestry caller; no parallel
  linear-versus-SSZ runtime selector or fixed anchor-gap threshold.
- Reuse etna-core L1HeaderView for original RLP identity and four fields. Decode
  Q's EIP-4788 field 19 only as a thin consumer adapter with existing Alloy RLP;
  never reconstruct headers or replace the authoritative general codec.
- Select traversed SSZ schemas from the authenticated beacon header's own slot
  and checked-in official mainnet/Sepolia/Hoodi schedules keyed by statement
  chain ID; no host-defined generalized paths or network profile. Reject an
  unknown chain/schema and an unrecognized authenticated fork version.
- Share canonical proof nodes within their authenticated tree; local indices
  in different roots never share one unscoped node map. Bound inputs before
  maps/allocation, reject duplicate/conflicting/unconsumed helpers and periods,
  and compare every reconstructed subtree against its authenticated parent.
- Existing raw-header size bound is 4096 bytes. Use the caller's authenticated
  deployment/profile budgets for headers and aggregate proof nodes; do not
  treat host-selected budgets as final profile authority.
- Cached native checks use the existing shared target, offline isolated
  snapshots and two build jobs. After measured cache-only runs, root allowed a
  5.75 GiB floor with a stricter 128 MiB per-run growth budget for only the
  protocol/composer test and lint packages. Other/broader checks retain the
  6 GiB floor; stop at either bound. No full guest/SDK/ELF/image build,
  cleanup, deployment, registration or chain write is part of this batch.
- Normative PROOF-6.2-33/34 and BATCH-6.3-5..7 reconciliation remains a separate
  activation gate. A tested native component is not complete SGX acceptance.

### Task 9: Native Q-bound sparse Ethereum history and composer

**Files:**
- Create: crates/protocol-etna/src/l1_history/mod.rs, merkle.rs and tests.rs.
- Modify: crates/protocol-etna/src/lib.rs and Cargo.toml (existing sha2 only;
  test-only serde_json if required by public fixtures).
- Create: crates/protocol-etna/tests/fixtures/l1-history/ public vectors and
  README.md with input provenance, hashes, exact Q bytes and adaptation limits.
- Modify: crates/guest-common/src/etna.rs and etna/tests.rs.
- Modify: crates/guest-common/src/lib.rs to remove the CI-confirmed unused bail
  import, with no behavioral change to Unzen.
- Reconcile: root and standalone guest Cargo.lock files without SDK/pin drift.

**Interfaces:**
- Export native borrowed L1HistoryProof, L1HistoryLimits, AnchorHeaderWitness,
  BeaconHistoryWitness, BeaconHeader, HistoricalPeriodWitness and SszProofNode
  from protocol-etna::l1_history. Limits contain max_headers/max_proof_nodes;
  they are caller/profile policy, not authority inferred from the witness.
- L1HistoryProof contains reference_header bytes, sorted unique anchor openings,
  optional beacon-history material only when anchors precede Q, and limits.
  Each opening contains original raw header bytes and, below Q, the target's
  BeaconHeader plus execution-payload hash siblings. Historical-period groups
  contain their period, block_roots root, legacy state_roots root only where
  needed and a root-scoped vector multiproof. Reference state has one shared
  helper set and authenticated historical list lengths. Paths are derived.
- Export verify_l1_history(chain_id: U256, reference_number: u64,
  reference_hash: B256, proof: &L1HistoryProof<'a>)
  -> Result<Vec<L1HeaderView<'a>>, L1HistoryError>. All supplied openings must
  verify before any authenticated views are returned. Source reference fields
  directly from the same LandStatement used for the final digest.
- Guest-common exposes the borrowed proof at its current seventh argument,
  retains its public L1AnchorProof name as the integrated view, and binds every
  projected anchor and carried raw witness to returned authenticated views.
  Keep FI guards and genuine KZG/blob/payload equality in the same composer.

- [x] **Step 1: Write intended rejection tests and a semantic RED.** Cover a
  forged Q raw hash/number, detached root, wrong slot/version/schema, missing
  anchor, changed raw carried header with matching number/root, malformed
  helpers/extra nodes and resource counts before parsing. Preserve old real-DA
  and FI tests by supplying valid SSZ fixtures, not weakening their assertions.
  Compile RED using a frozen exact-source native harness before implementation;
  failures must reach the new relation rather than fixture setup errors.
- [x] **Step 2: Implement canonical SHA-256 multiproof verification.** Derive
  minimal helper positions from required leaves; check positive u64 generalized
  indices, antichain/duplicate/overlap rules and exact root. Fold only correctly
  scoped trees. Prove reference state.slot and scheduled Fork.current_version
  under the authenticated reference BeaconHeader's state root.
- [x] **Step 3: Implement header-history relation.** Check exact original Q
  bytes, number/hash and parent beacon commitment before using any metadata.
  Recompute each target BeaconHeader and authenticate its execution block_hash
  from its body under the target slot's Bellatrix/Capella or Deneb+ schema.
  Anchors equal to Q use exact raw identity, not an unrelated beacon opening.
- [x] **Step 4: Implement all history positions.** With W=8192, R=reference
  beacon slot and target actual slot s: s=R uses reference beacon identity;
  0<R-s<=W uses block_roots[s%W]. Older completed periods use summaries or
  legacy roots, never the current partial period. Authenticate list lengths;
  C=R/W, first_summary_period=C-summary_len and index=target_period-first.
  Legacy index=target_period; open HistoricalBatch(block_roots,state_roots).
  Check both lengths/period arithmetic and their expected network relation.
- [x] **Step 5: Integrate the existing native composer.** Validate every supplied
  opening and every matched block; use exactly the statement's chain/Q context.
  No host callback, replacement root flag, new trie, execution or transport.
- [x] **Step 6: GREEN and focused lint.** Run the full exact-source native suite
  and protocol tests with public recent/completed-period witnesses where full
  input is recoverable; label any synthetic tests. Run strict scoped all-target
  Clippy, actual formatting, locked metadata and lock/package-identity checks.
  Commit only after complete diff review and independent review/verification.

### Task 10: Independent behavioral verification and source review

**Files:** Task-9 complete diff, native/test fixtures, plan and design; scratch
verification reports stay in this plan's ignored workspace or temporary outputs.

**Interfaces:** Consume the actual full diff and original explicit-Q requirement,
not only the implementer's explanation. Produce independent source verdict,
behavioral results, exact source hashes and remaining native-versus-runtime limits.

- [x] **Step 1: Challenge proof scope and tree composition.** Independently
  mutate Q, public statement, target header, beacon root, fork, slot, period,
  list length, node position/value and carried/inherited opening. Keep shape
  valid where needed so each intended predicate is actually reached.
- [x] **Step 2: Exercise protocol boundaries.** Cover Q=L with earlier anchors,
  reference-parent payload itself, W/W+1, exact rollover, zero summaries,
  last legacy/first summary, missed slots, body/state fork transitions, sparse
  anchors, shared helpers, upper u64 bits and declared resource limits.
- [x] **Step 3: Verify integration controls.** Retain all real KZG/DA and FI-off
  regressions and compare Unzen policy/source seams. Read current CI failures
  separately from source outcomes; do not call RISC0 artifact 403 a passing
  full build or hide it with a bypass.
- [x] **Step 4: Resolve confirmed findings and recheck.** Use independent source
  review and behavioral verification, then publish/read back the exact draft
  head and update plan/Notion. Keep SGX/ZK/ELF/provenance/upstream activation
  limitations explicit. No merge or deployment.
  Published source checkpoint `71152347` and its exact draft head/body were
  read back; the Notion anchor-linkage page was updated without warnings.
  New-head CI remains separate from the recorded native validation.

## Target amendment: explicit lander-selected Q (2026-11-27)

The user replaced the earlier optional-Q/outer-attachment direction with one
explicit Q for every proving job. The lander chooses Q and supplies witnesses;
Raiko2 verifies the Q-bound relation without reference-selection policy. This
supersedes the target selection/attachment policy in P03-A02/A04 and the older
recovered near/far prose. The native SSZ checkpoint above now supersedes the
2026-11-24 Etna linear caller; full activation gates remain open.

- [x] **P03-A09a — Record target:** keep L as the fixed synchronized last-anchor
  boundary and Q as a separate authentication reference with L<=Q. Always supply
  Q explicitly; Q=L is valid when usable. Bind Q number/hash in the existing
  statement shape. Remove None mode and the outer optional attachment target.
- [x] **P03-A09b — Complete native uniform authentication:** verify the raw Q header
  against its statement hash, authenticate earlier distinct anchors with SSZ
  and shared witness nodes, and use direct header identity for anchors equal
  to Q. Specify beacon block/state/history paths, execution hash/raw-header/
  number/root mapping and fork/slot/list boundaries for required anchor facts.
  Retain monotonicity, inheritance and generation-origin exceptions.
  Native anchor-history relation is tested; other required L1 predicates retain
  their own pending authenticated-context tasks.
- [ ] **P03-A09c — L1 Q authentication:** obtain H(Q) through the actual enabled
  history interface, testing ages 1/256/257/8191/8192 and activation. Reject an
  unavailable Q. Check actual inclusion, not broadcast time. Do not substitute
  newer synchronized state for the blocks' real last anchor L.
- [x] **P03-A09d — Lander ownership and rebinding:** the external lander selects
  Q, prepares witnesses and requests the Q-bound proof; Raiko2 has no automatic
  choice/switch/refresh/expiry-retry behavior. Test inconsistent Q/raw-header/
  statement bindings, root substitution, wrong heights and a changed Q paired
  with the old proof. Changing Q requires a new proof binding, not merely new
  witness bytes; execution-child reuse is unestablished and not promised. Native
  statement/raw-header binding and absence of Q-selection policy are checked; external lander deployment and
  expired-Q onchain acceptance tests belong to A09c, not this native component.
- [ ] **P03-A09e — Protocol/interface review:** reconcile PROOF-6.2-33/34 and
  BATCH-6.3-5..7, retain existing statement fields where appropriate, specify the
  SSZ witness and resource bounds, and measure costs before runtime activation.

Require 1<=P-Q<=8191 at actual landing under enabled/data-available EIP-2935,
or <=256 for BLOCKHASH alone. An expired Q must be handled by the lander with
another Q-bound proof; an unqueryable Q never becomes an accepted fallback.
The state boundary L stays fixed. This does not revive old-generation or
discarded statements after governance recovery. Existing QC/FI implementation
holds, normative SSZ activation gates and SGX-first priorities remain in effect.

## L1 root authentication reuse (2026-11-24)

The user authorized completing Etna root authentication using Unzen's existing
trusted-reference/header-ancestry pattern. This is not a new Anchor transaction
or contract interface. Keep the existing EVM system-call implementation; bind
its supplied root to L1 before returning the native statement result.

- [x] **P03-A08a:** reuse #22426's exact portable raw-header codec extraction on
  the existing F31 baseline, preserving all other core and QC behavior. Review
  and test the narrow source delta independently before selecting an immutable
  development pin. Do not repin to the unrelated committee/parameter changes
  in the full candidate. Keep the upstream full-client merge gate separate.
- [x] **P03-A08b:** factor only Unzen's consecutive header hash/number walk at
  the existing guest-common seam; retain its event origin, timestamp, source,
  stalled-anchor and parent-checkpoint policies. Etna uses the same walk with
  hashes of original header bytes and the statement's reference number/hash.
- [x] **P03-A08c:** at the existing native Etna composer, bound
  the raw chain before decoding, open carried AnchorWitness header bytes using
  existing RLP, and match each covered block's anchor number and PBBR to that
  authenticated chain member. The same chain supplies the incoming inherited
  anchor opening; no extra parent witness or host root/success flag. Preserve
  generation-origin parent exceptions, FI-off restrictions and real KZG/DA.
- [x] Verify valid future-header extensions, altered references/ancestry/root/
  witness bytes, inheritance and bounds, including independent regression
  controls and source review. No guest/SDK build or new target/profile.

This scoped source adoption replaces the earlier all-or-nothing use of #22426
as a global root-verification blocker. It does not merge that PR or complete
full client verification. The native borrowed proof view does not freeze a
serialized SGX input or final ABI. Reference authority still comes from L1's
reconstructed statement; Inbox MPT facts, exact time/age/rules, committee policy,
execution and final SGX acceptance remain separate obligations.

### Root-authentication acceptance checkpoint

The required borrowed `L1AnchorProof` is checked at the existing native
`verify_blob_range_statement_projection` caller. The same statement supplies
both expected reference fields; checked u64 conversion rejects upper bits.
Every raw header participates in a bounded consecutive hash/number walk. Each
covered block's PBBR matches its anchor member's state root; each carried box
opens exactly that member's original raw bytes. Incoming inheritance uses the
same chain opening, without a second host root or parent witness argument.

The private walk is shared with Unzen after its existing policy checks. Its
stalled/source/parent-checkpoint/time behavior is retained. One multiply-invalid
input may report a checkpoint error before a simultaneous link error; both
reject. Number wrap now rejects through checked addition. Header hashing is
not duplicated by the shared walk.

Current source is [4827fea0](https://github.com/taikoxyz/taiko-mono/commit/4827fea0e844236d14cceafd6bd84a820bff2dd0),
a direct F31 child replaying exactly #22426's five-file extraction. All 30 other
core files, including F31's committee/parameters/ValState and GRID_INTERVAL,
remain byte-identical. ABCI uses the same parser. This is a reviewed development
source carrier; upstream full-client merge and durable R06 provenance remain
open. It does not adopt the newer committee or execution implementation.

Acceptance evidence:

- Semantic RED compiled with 45 maintained cases passing and seven intended
  rejection cases failing at the ignored native seam; all positive fixture
  setup paths worked. Two mutable-slice setup errors were fixed before that
  RED. The incoming-parent fixture's optional fields were corrected for the
  existing canonical round-trip check before final GREEN.
- Against the published Git revision, the exact-source native harness passed
  **64 adapter + 132 protocol tests**. The adapter includes 52 maintained and
  12 independent tests, restoring all seven earlier independent cases with
  actual raw openings. The independent shared-walk oracle compares 14 real
  Alloy Header/checkpoint cases using the original and refactored Unzen
  post-policy bodies. Future opaque fields, exact-Q/fork/tail substitutions,
  malformed boxes, resource bounds and both inheritance paths are covered.
- Strict scoped all-target Clippy and independent source/behavior review passed.
  Two item-scoped lint expectations keep the helper crate-private: the compiler
  rejects plain pub via unreachable_pub while Clippy flags restricted visibility
  within the private module. No verification check or broader lint is disabled.
- The source carrier passed 51 core unit tests, 27 original-parser adapter/oracle
  tests and strict all-target Clippy. Root and both standalone default/bench
  locked metadata resolve. Package counts remain 1,208 / 561 / 602; only the
  etna-core source identity changes, plus the normal guest-common core edge.
  Existing SDK/crypto pins are unchanged. Actual workspace formatting, locked
  offline metadata and diff whitespace checks pass.
- The diagnostic harness compiles identical product/crypto/codec sources with
  small transport/error support, not the complete production guest stack.
  Full guest-common library checking stopped before Cargo at the 6 GiB disk
  reserve, so it is not a passing compile claim. Source collection and small
  native checks reused the shared target with resource caps. No new target,
  SDK/SGX/ZK build, ELF edit, cleanup, registration, deployment or chain write.

The caller still supplies L1 authority for the statement reference, Inbox
account/storage proofs, exact anchor time/age/rules, authenticated profile,
consensus and actual execution. This closes native reference/anchor-root
binding, not the complete SGX verification/landing relation.

## Internal-testnet acceptance: SGX first, ZK later (2026-11-23)

The user explicitly chose a single-SGX first internal-testnet milestone. Do not
require SP1/RISC0 proofs or the production two-subproof policy before accepting
this milestone. This supersedes the earlier native/SP1 priority and the
phase-one requirement below to complete both ZK guests. Later ZK integration
must consume the same complete shared verification relation and statement.

The first acceptance path is real range collection -> complete native/enclave
Etna verification -> SGX signature -> internal-testnet verifier/landing receipt.
No partial projection digest, host success flag or signature over unchecked
execution/context can substitute for the complete verification relation.
QC, anchor/context authority, phase-one FI-off profile and source adoption gates
remain as recorded; choosing SGX does not remove these checks.

- [ ] **SGX-01 / P04–P05:** compose the complete actual Etna range verifier,
  including existing EVM replay/state-root, DA, anchor/context/rules and adopted
  consensus checks. Reuse it unchanged in native and SGX paths.
- [ ] **SGX-02 / P06:** connect an Etna input/response path to the existing SGX
  runtime, provider and pipeline. Preserve Shasta behavior and range/task
  identity. Sign only after complete verification succeeds.
- [ ] **SGX-03 / P07:** align Etna statement, verifier-domain hash and proof
  bytes with the actual internal-testnet SGX verifier. Reuse existing signer,
  quote and proof-format infrastructure where compatible; do not pass an Etna
  digest through the Shasta carry-data hash relation.
- [ ] **SGX-04 / P07–P08:** validate the new enclave measurement and instance
  registration, and exercise successful range landing plus altered execution,
  DA, context and signature rejection on the selected internal-testnet fixture.
  Actual deployment, registration and chain writes need their own execution
  authorization; this plan does not perform them.

The existing mono `AnyVerifier` can accept one SGX_RETH subproof. It is a
candidate for an explicitly scoped internal-testnet verifier configuration,
not a claim about what the current testnet runs. Check the actual pinned Etna
Inbox/verifier wiring at integration. Keep the SGX-only acceptance policy in
that testnet verifier/deployment configuration; add no host-controlled proof
bypass and do not change the production verifier policy for this milestone.

SP1/RISC0 guest builds, ELF/VK refresh and ZK proof acceptance remain unchecked
later tasks. The SGX milestone does require an actual enclave artifact, valid
attestation/registration and matching onchain signature acceptance; a native
fixed-signer run is only an earlier diagnostic.

## Continuous closeout authorization (2026-11-23)

The user authorized completing all remaining independent work without pausing
after each three tasks. This supersedes the checkpoint cadence below for future
work; preserve its historical completed batches. Stop only for a real design
choice or unmet dependency, keeping settled native checks separate from the
known QC/FI/profile, execution-source/compiler/API, anchor-source and final
proof-interface gates. RISC0 stays lower priority.

The current independent closeout scope uses existing public/native consumers:

- [x] **P05-PH1:** restrict the existing native authenticated range/statement
  composer to phase one: absent FI witnesses and unchanged before/after cursor.
  Reuse the checked monotone cursor sequence instead of duplicating it. This
  does not authenticate the disabled/empty-queue deployment profile. Keep the
  generic protocol projection and all statement/envelope fields FI-capable.
- [x] **P03-A05d:** require present/nonzero carried anchor roots for generation-
  first and advancing covered blocks in the existing range verifier. Preserve
  inherited-root and witness-error behavior and the excluded origin-parent
  exception. No L1 parser/root authentication or storage read is added.
- [x] Complete the independent remaining-work audit, relevant regressions and
  adversarial/behavioral review; publish one consolidated status and distinguish
  these component checks from full Etna execution/proof acceptance.

### Independent closeout acceptance

The existing native composer now rejects FI witnesses at every covered height
and any before/after cursor change, after the generic projector validates the
complete monotone sequence and its cuts. Nonzero and maximum uint48 constant
cursors remain valid. The generic protocol range/statement checker remains
FI-capable. The existing range verifier also rejects absent/zero carried roots
at generation-first and advancing blocks; inheritance and excluded-origin-parent
semantics remain intact. These checks authenticate no deployment or L1 root.

Acceptance evidence for the exact reviewed product bytes:

- Compiled semantic RED reached both FI rejection cases with otherwise valid
  real KZG/range/statement inputs, and both carried-root rejection cases. The
  nonzero/max cursor positive passed before implementation.
- The frozen native component run passed 50 adapter and 132 protocol tests,
  including three new independently authored controls. Strict all-target
  Clippy passed for both harness packages. The harness compiles byte-identical
  product code and the existing crypto/codec, with small diagnostic transport
  and error support; it is not a production guest or full-workspace build.
- Independent adversarial source review and behavioral verification passed.
  Tests include valid cut/statement cursor consumption, optional FI witness
  presence even with opaque empty/malformed contents, exact carried-root error
  heights, incoming and interior advances, and the opaque origin-parent case.
- An old inheritance fixture with an invalid carried prefix now rejects earlier;
  the maintained inheritance case uses a valid prefix, while all seven incoming
  excluded-parent root combinations remain covered. A historical scratch case
  also accepts the correct earlier typed rejection. No production check was
  relaxed. The shared root predicate avoids duplicate conditions/lint exceptions.
- Actual workspace formatting, locked offline metadata and diff whitespace
  checks passed. Dependency pins, manifests/locks, SDKs, EVM/trie, guest inputs,
  final ABI and generated artifacts are unchanged. Checks used the existing
  shared target, 11 GiB disk reserve and 256 MiB per-run growth cap; no new
  target/profile, SDK/guest/ELF build, cleanup, merge or deployment occurred.

The remaining-work audit found no additional current-pin implementation slice
with both a settled rule and usable existing consumer. Do not substitute unused
helpers for the actual deferred integration:

| Remaining group | Actual prerequisite |
| --- | --- |
| Strict full-body execution/range replay | Reviewed compatible canonical Alethia source, host compiler compatibility, retained legacy Unzen API and trusted Etna activation |
| Anchor/context/time/rules/stop authentication | Native root linkage is complete under A08; authoritative L1/profile inputs, Inbox facts and exact time/rules remain. F31's node-only stop grid predates the current timestamp stop rule |
| QC, FI phase two, distant-anchor bridge and final committee ABI | Existing design holds; phase-one disabled/empty-queue profile authentication remains required |
| Provider/pipeline/guest output, aggregation and genuine proofs | The complete verified relation and concrete authoritative input/output first; then native/SGX internal-testnet acceptance; ZK follows later |

These completed guards do not close P01–P08, authenticate a profile or establish
full execution/proof acceptance. Full workspace Clippy and actual guest/proof
builds were not run in this bounded component check and remain delivery gates.

## Three-task checkpoint cadence (2026-11-22)

The user authorized continuing the remaining settled work in groups of three
completed tasks. Pause after each group and summarize its results and review
needs. Pause sooner on an external dependency or a material implementation
choice requiring confirmation; do not work around an unmet source gate or count
planning/status checks as completed implementation tasks.

### Batch B01 — Ordinary anchor verification

1. **B01-1 / P03-A05c2 — Shared-header source gate:** obtain review and the
   required full native-client verification for taiko-mono #22426, then verify
   compatibility with Raiko2's F31 baseline before choosing an immutable pin.
   Keep unrelated QC/committee changes outside adoption during the design hold.
2. **B01-2 / P03-A03, P03-A05c3 — Near-reference anchor composition:** bind the
   carried raw headers to consecutive L1 ancestry ending at the authenticated
   reference and to the same matched EL anchor numbers/roots. Preserve the
   excluded-parent opening only when the first covered block inherits. Reuse
   useful existing linkage primitives at the actual caller.
3. **B01-3 / P03-A05c3 — Anchor time and age composition:** check the exact
   timestamp maximum, parent-time overflow and authenticated age/origin limits
   at that same range consumer. Keep checkpoint storage predicates on the
   existing proof engine; do not add a separate generic verifier.

**Early checkpoint: B01-1 remains incomplete, zero implementation tasks
completed in this batch.** #22426 is OPEN/DRAFT at `8268eb63`, with no submitted
reviews or comments. Its automatic draft checks skipped full client validation;
the existing [manual full-client workflow](https://github.com/taikoxyz/taiko-mono/actions/runs/45017187106)
was then dispatched and read back at that exact head. Native unit, lint,
EL-import, full Docker lanes and checkpoint-resource jobs actually started;
lockfile and lander artifact reproduction passed at the recorded intermediate
snapshot. The completed run subsequently failed the supervisor and both Docker
lanes, while native unit/lint, EL-import, resources, lockfile, artifacts and
signer checks passed. The supervisor fixture failed before the Rust parser;
devnet expected an absent FI witness; native checkpoint import encountered
peer admission refusals. Exact-base reproduction was not performed, so this is
not a blanket claim that all failures are unrelated. Manual dispatch skips the
separate production-image job. The scoped 58 core / 27 adapter tests remain
component evidence, not a replacement for the red full-client gate.

The F31-to-candidate source audit also found unrelated API/behavior changes in
committee selection, parameters and ValState. In particular, core removed
`parent_el_hash` and its bare app-hash check from `verify_body_at_origin`, while
Raiko2's deprecated historical helper still calls the four-argument form.
Blindly removing that argument would weaken that helper's documented relation;
no candidate Raiko2 build or compatibility fix is claimed. The core manifest
itself is unchanged; node-workspace Alethia pin changes are not automatically
Etna execution adoption for Raiko2.

At that checkpoint Raiko2 remained at F31. A08 now adopts the narrow compatible
codec carrier and completes native B01-2 root linkage; B01-3 timing still depends
on authenticated context. The earlier batch depended on source-compatible adoption
and must not substitute a duplicate parser, unchecked host timestamp/root or an
unreviewed pin. Keep QC-dependent behavior outside this adoption while its design
is paused. Resume after the source gate is met or the user explicitly changes
that gate. FI, QC management, distant-anchor SSZ and final committee-dependent
ABI remain on their existing holds.

## DA derivation scope amendment and batch B02 (2026-11-22)

The user clarified the proof relation: the guest derives the proposal-bound
public blob bytes successfully, consumes the complete input, compares the
complete decoded payload with the exact proven block range, and proves its
execution/state-root relation. Any failure terminates verification and cannot
produce an accepted proof. Host success flags, partial/default payloads and
successful execution of only a prefix are not substitutes.

Reuse the existing blob codec and a mature zlib implementation. Keep the
protocol's v6 framing, complete-stream/exhaustion and byte/size constraints;
checks already implemented by the selected decoder are not reimplemented.
Do not create a custom DEFLATE parser, malformed-input recovery, fallback
blocks, or a separate general-purpose decoder-hardening/conformance project.
Prover timeout/OOM handling is separate from the proof relation. The decoder's
success semantics must still agree with public derivation; known inconsistent
backends cannot be selected merely because they return success.

B01 ordinary-anchor adoption remains pending independently. It does not block
this batch, which preserves current source pins, EVM/trie/KZG and QC/FI holds:

- [x] **B02-1 / P05-D02 — Blob packing adapter:** at the existing guest-common
  Etna seam, delegate to the already-used `BlobCoder`, enforce the Etna 1..6
  blob limit, first field-element high bits and full non-final chunks, and
  concatenate in supplied order. Do not implement another codec or KZG engine.
- [x] **B02-2 / P05-D01b — Complete v6 derivation:** use the existing flate2
  dependency/backend to decode one v6 zlib stream; require successful end and
  complete input consumption, retain the 64 MiB protocol limit and fail directly.
  Confirm the actual selected backend rather than assuming default-only miniz.
- [x] **B02-3 / P05-D03 — Payload/range composition:** compose those checks
  with `verify_decompressed_execution_payload` in the existing protocol crate,
  preserving complete ordered execution-item equality and error propagation.
  Add focused positive and altered/truncated/extra-input regression cases and
  independent review/behavior checks before publishing the bounded component.

The production guest entrypoints and full range execution are not wired by
these native DA components. Existing KZG verification must authenticate the same
ordered blobs and statement hash list at final composition; this batch does not
accept a host-provided KZG/decode/execution success flag. Keep the portable
protocol-etna closure intact and place adapters over existing std-dependent
guest codecs in guest-common. Final backend builds/proofs remain P07/R07.

B02 native-component acceptance evidence:

- Each task had a compiled semantic RED: 8 packing failures; then 8 existing
  pass / 7 stream failures; then 15 existing pass / 5 composition failures.
  Failures reached explicit implementation seams, not setup errors.
- Frozen exact-source adapter tests passed **20 maintained + 4 independently
  authored cases**. The harness compiled byte-identical guest-common code and
  the pinned client `06bd6b0` BlobCoder source, without its service stack. Tests
  cover real complete range projection and signed transactions, stored/dynamic
  alternate streams, complete input/trailers/checksum/dictionary failure,
  exact 64 MiB/+1, one/full/six blobs, padding/order and deep typed equality
  failures. Independent fixtures manually pack blobs and stored streams.
- Scoped `cargo clippy --locked --offline -p etna-da-adapter-check --all-targets
  -- -D warnings` passed after checked counter conversions. Existing Etna
  regression tests passed: 120 maintained plus ten inherited independent-parent
  controls. Independent adversarial source review and behavioral verification
  passed at the final source bytes.
- Actual production-workspace `cargo fmt --all -- --check` and full
  `cargo metadata --locked --offline --format-version 1` passed. All 1,208
  package identities/sources/versions/checksums remain unchanged; the root lock
  adds only guest-common's four dependency edges. The direct Alloy EIPS 1.8.3
  edge matches the pinned codec's builder trait; flate2 explicitly selects the
  already-present C-zlib feature. The pure protocol-etna sources are unchanged.
- The native harness linked system `libz.so.1`; feature selection alone is not
  final target-library identity evidence. The complete guest-common build and
  workspace Clippy, standalone guest lock refresh, both guest targets, ELF/VK/
  provenance, cycle measurements and genuine proofs were not run. Standalone
  guest locks still predate the new Etna dependency closure (including RISC0's
  existing flate2 version difference). Keep those explicit P07/R07 gates.
- Whole diagnostic-workspace formatting reported differences in the preserved
  upstream codec and scratch edition formatting; actual production-workspace
  formatting passed. Diagnostics are not product code. Checks reused the
  existing shared target with bounded jobs/disk growth; no SDK/ELF build,
  duplicate target, cleanup or deployment occurred.

The three B02 native tasks are complete; this is the requested checkpoint,
not completion of P05, the final guest relation or #305.

## Batch B03 — Same-blob authentication and statement binding (2026-11-23)

Continue independently of B01's source gate and the QC/FI/distant-anchor holds.
Reuse the existing InputDataSource, primitive proof-of-equivalence verifier and
versioned-hash helper; no new transport, cryptography, EVM or generic framework.

- [x] **B03-1 / P05-D04a:** authenticate 1..6 positional blob/commitment/proof
  tuples against the supplied hash list using existing real KZG helpers. Check
  exact counts and 48-byte fields; reject legacy inline/transport skip flags.
- [x] **B03-2 / P05-D04b:** pass those exact same authenticated blob bytes to
  B02's complete derivation and full projected-payload equality checks.
- [x] **B03-3 / P05-D04c:** compose the existing range/statement verifier and
  authenticated DA, using `statement.blob_hashes` directly. Return the original
  projections/digest only after both succeed; add no independently supplied
  digest or alternative blob list. Use the existing native execution transaction
  type rather than adding an abstraction just for codec-version traits.

Acceptance needs real commitments/equivalence proofs and the real existing Rust
verifier in positive and altered/order/count/payload/statement tests, independent
review/verification, and no package/revision drift. The existing `kzg-host`
feature may be enabled on the test dependency for fixture generation only.
A host callback or mock success is not evidence of KZG binding. Full execution,
L1/committee/rules/time/FI authority, final guest entrypoints/locks/ELFs and real
proofs remain separate. Pause after these three components or earlier on an
actual blocker or decision requiring the user.

B03 acceptance checkpoint:

- Real-crypto semantic RED stages compiled and reached their explicit seams:
  20 prior pass / 9 new failures; then 29 / 4; then 33 / 7. GREEN preserved
  those controls. Final native source tests passed **40 maintained + four
  independently authored cases**, including actual native TransactionSigned,
  real c-kzg-generated equivalence proofs, unchanged Rust kzg-rs verification,
  pairing-reaching substitutions, authenticated wrong payloads and changed
  statement hashes/cuts/endpoints. Scoped strict Clippy passed after two
  test-only semicolons. Existing protocol regressions passed (120 maintained
  plus ten inherited independent-parent controls).
- Independent source review and behavioral verification passed. The narrow
  offline harness compiled byte-identical adapter and original crypto utility
  sources against the pinned real curve/setup libraries; its small diagnostic
  error/transport facades omit unrelated service types, not verification. It
  does not establish a complete guest build or cryptographic audit.
- Actual production formatting, root locked offline metadata and both guests'
  default/bench locked metadata passed. No root package/source/SDK pin changed;
  the only new manifest feature is test-only `kzg-host` on the existing dev edge.
- The prior head's ordinary Rust CI passed, including guest-common. Its xtask
  lane actually failed 43 pass / 5 fail while resolving stale standalone guest
  locks, before ELF freshness checks. Separate commit `ae0708c2` synchronizes
  those two locks: existing SDK/git/crypto pins preserved, new Etna closure
  aligned with root, RISC0 flate2 1.1.9 replaced by the already-required 1.1.8.
  Default and bench locked graphs pass independently for both. This closes the
  dependency-resolution prerequisite, not R07 artifact validity.
- New source/SDK target builds, static library/ELF/VK/source provenance and ZK
  proofs were not run. L1 authority, QC, anchor/rules/time, actual FI consumption,
  EVM state-root verification and final guest/public-output framing remain open.

The three B03 native components are complete. Stop at this batch checkpoint;
metadata repair is a separately reviewed prerequisite, not a claim of final
proof or main-merge readiness.

## B04 execution preflight and early checkpoint (2026-11-23)

No B04 implementation task is complete. The source preflight reached an actual
execution dependency, so pause this batch under the three-task cadence rather
than introduce an unused adapter or duplicate the execution engine.

The checked Raiko2 source is `fdb99fff91850652a1e10985d341538cc290ae33`.
Alethia main was refreshed at `fce87eff1d7b720852468e566a6df686b5487dbc`;
its hardfork enum and executor still contain no Etna branch. The current
Raiko2 execution pin `59e06d049beed8442218e7ab258138c060511c87` also predates
Etna. [Alethia #248](https://github.com/taikoxyz/alethia-reth/pull/248)
remains open at `ad584325c6b1e15f3c163fb25e71c04a78b194e0`.
Per the repository's canonical-source rule, adopt a reviewed compatible commit
from Alethia main after the required fork implementation lands there. Merely
relaxing Raiko2's anchor assertions or changing activation time cannot provide
Etna fee rules, system calls or anchorless execution.

The existing supplied-block path is already strict at its pinned dispatch:
`Executor::execute` calls OP-reth `BasicBlockExecutor::execute_one`, whose
ordinary path executes each committed transaction with propagated errors.
Alethia's legacy `prover` filtering override is reached through different
paths, including the state-hook path; it is not evidence that this ordinary
Raiko2 call filters transactions. Recheck dispatch after adopting the candidate's
new OP-reth revision. Preserve the existing supplied-block validator, signer
recovery, ancestor checks, WitnessDatabase, SparseState and post-state-root
check. Shasta candidate reconstruction retains its existing filtering semantics.

The next implementation batch, after canonical compatible source is available:

- [ ] **B04-1 / P04-E01 — Execution dependency adoption:** adopt the reviewed
  main revision and aligned OP-reth/Alloy closure without an SDK upgrade. Check
  native and guest dependency compatibility and trusted fork activation. Verify
  first-Etna fee behavior and real EIP-2935/EIP-4788 writes through the existing
  executor. Do not replace source provenance with a host activation flag.
- [ ] **B04-2 / P04-E02 — Fork-scoped supplied-block validation:** keep legacy
  zero-root, anchor and first-receipt assertions for Unzen. Apply the upstream
  Etna root/extraData and ordinary transaction rules at the same existing
  validator, selected from its trusted chain spec. Test empty blocks, ordinary
  tx0 revert/OOG, missing/zero PBBR, interior zk-gas overflow, missing system-call
  witness and changed receipt/header/state roots. Golden-touch transactions
  become ordinary transactions; do not add a new sender ban.
- [ ] **B04-3 / P05-D05a — DA-bound native range replay:** at the actual
  consumer, construct bodies from the exact matched header and original
  EIP-2718 spans using the existing codec, then replay every ordered block with
  its witness and existing ancestor rollover. Bind the first parent/history and
  terminal root to the same range before returning an execution result. Test
  count/order, parent/history and later-witness substitutions. This native
  integration does not close QC, context/anchor authority or final guest output.

Current CI for `fdb99fff` has green ordinary Rust lanes, but two artifact
acquisition failures prevent full acceptance:

- [xtask](https://github.com/taikoxyz/raiko2/actions/runs/45104591258/job/137462917945)
  stops while compiling host dependency `risc0-circuit-recursion 4.0.4`: the
  pinned recursion archive returns HTTP 403. Tests, locked-metadata assertions
  and guest provenance checks were not reached. The existing build script can
  accept an independently provisioned checksum-matched archive through
  `RECURSION_SRC_PATH`; finding a local copy does not provision CI.
- [CUDA image](https://github.com/taikoxyz/raiko2/actions/runs/45104591472/job/137462844574)
  stops in existing `rzup 0.5.1` installation of `risc0-groth16 0.1.0`:
  distribution metadata returns S3 AccessDenied. The parallel host build is
  cancelled. The relevant installer/workflow and pinned dependency inputs are
  unchanged from the comparison base; no base rerun or endpoint root cause is
  established. Restore official artifact access before rerunning these gates.

Do not bypass downloads, SDK checks or provenance, or hand-edit ELF metadata to
make these jobs green. No SDK/guest build, proof, cycle measurement, dependency
adoption, cleanup or source implementation was performed at this checkpoint.
B01 still waits for #22426; QC/FI/distant-anchor/final committee ABI holds remain.
B02/B03's authenticated DA components remain complete; their execution consumer
and final proof acceptance do not. These are separate dependencies, not a claim
that the whole design or native component verification failed.

## Native/SP1 priority and candidate preflight (2026-11-23)

The user lowered RISC0 priority. Keep its artifact-download, compiler and proof
work as eventual R07 gates, but do not put them ahead of independent native/SP1
integration. This changes scheduling, not final backend support or acceptance.

The [candidate compatibility checkpoint](../../research/2026-11-23-etna-execution-candidate-compatibility.md)
records three completed preflight investigations, not completed B04 implementation:

1. Reproduced the pin-only Cargo conflict, then resolved the isolated candidate's
   aligned Alethia/OP-reth/Alloy native graph without changing production pins.
2. Independently passed native and SP1 default/bench locked offline metadata;
   existing SP1 SDK/guest and crypto patch identities remain unchanged.
3. Verified the exact supplied-block API boundary and ran native compiler
   preflight. The candidate declares Rust 1.99, so the pinned host 1.95 check
   stops before compilation. Source inspection also found the removed delayed
   Unzen clone method still used by Raiko2. The Etna setter is not equivalent.

The next actual execution work needs canonical source adoption, host toolchain
compatibility and preservation of that legacy Unzen API/behavior. A temporary
candidate graph is not authority to pin an unmerged source, bypass native MSRV,
rename the old setter or remove Unzen support. SP1 compilation remains untested;
metadata and declared MSRV alone establish neither success nor an SDK-upgrade
requirement. B04-1/2/3 remain unchecked. No RISC0 build was attempted.

## QC design hold and non-QC continuation (2026-11-22)

The user directed that QC/committee work pause pending the revised design,
which is expected to manage membership changes, penalties and queries on L2.
Reusing the node implementation is a candidate after that design settles, not
an adopted interface. Do not extend or freeze the old L1 committee-authority
proposal, registry proof adapters, committee derivation or QC policy. Keep the
existing tested helpers as partial components without wiring them into a final
proof. This amendment supersedes earlier instructions to complete that path.

- [ ] **QC-HOLD:** once the revised design is stable, inspect the authoritative
  node/L2 contract APIs and decide what to reuse or verify. P03 committee and
  P03-L01..L04 authority work, the fewer-QC discussion and final committee-related
  statement wiring wait for that review. #22423 is a historical provisional
  discussion and is not a blocker for independent execution/DA work.
- [x] **P05-D01a:** bind the complete decompressed execution-list bytes to the
  original matched item spans, with exact canonical outer RLP framing and the
  inclusive 64 MiB bound. Retain borrowed spans and existing codecs; no second
  expected payload, dependency or transaction/header decoder.
- [x] **P05-D01b:** reuse the selected mature decoder for complete v6
  derivation and bind its entire output to D01a, through the B02 native
  guest-common adapter. A failed derivation directly fails verification; no
  default manifest, partial execution or decoder-recovery branch is added.
- [x] **P05-D04:** compose existing KZG helpers with the same ordered blob
  bytes, B02 derivation and the existing native range/statement projection.
  B03 uses the statement's exact hash list and returns its digest only on success.
- [ ] **P05-D05:** wire this native composition into actual guest entrypoints
  with authoritative context, complete execution and final public output.
  Native DA binding alone is not full proof acceptance.

The earlier miniz-only streaming proposal was not adopted because its success
semantics differed from public derivation. That source-review result remains
historical evidence for backend selection, not a requirement to write a custom
strict inflater. Inspect the real dependency feature union: the pinned client
protocol already enables flate2's C-zlib backend in the existing guest closure.
Reuse its built-in checks and a thin complete-stream wrapper. No separate RFC
hardening campaign or new decoder implementation is part of the accepted scope.

The current non-QC payload source was inspected at taiko-mono main
`3602629758beb653423a23608fd1960810efcfac`: one version byte, one dictionary-free
complete zlib stream, no trailing bytes, canonical RLP list items equal to the
projected range, 64 MiB output and 124000 bytes per item. This remains a pinned
component baseline, not a claim the evolving overall design is frozen.

## Pending decisions and independent work (2026-11-22)

Keep design/upstream dependencies separate from engineering tasks not yet done:

| Item | Disposition | Independent work that can proceed |
| --- | --- | --- |
| FI consumption and skip trials | Phase two deferred; authenticated FI-off profile remains required for phase one | Preserve cursor/witness slots and existing execution architecture |
| QC management and committee authority | User hold pending L2-managed design | Retain tested helpers; do not extend the old L1 proposal or freeze replacement interfaces |
| Final ABI, public inputs and verifier framing | Committee-dependent fields wait for the new design; the present codec is provisional | Preserve existing byte vectors; complete interface-independent body checks |
| Etna execution source/profile | Alethia #248 remains open at `ad584325`; Raiko2's current `59e06d0` pin has no Etna fork | Audit exact strict-execution, system-call and legacy-anchor adapter boundaries; adopt a reviewed compatible revision before final execution integration |
| Distant-anchor SSZ bridge | Historical strategy still needs normative reconciliation with the linear header-chain rule | Ordinary per-range anchor structure and the near-reference path do not require enabling the bridge |
| Complete DA derivation | B02/B03 native derivation and KZG/range binding complete; actual execution and guest consumer remain | Reuse the completed components at B04-3/P05-D05; no custom decoder or idle parallel adapter |
| R06/R07 and production proof acceptance | Final delivery gates, not additional protocol design decisions | Reuse shared target; rebuild final ELF/VK/provenance and run real proofs after the complete source/interface is settled |

Completed bounded component **P03-A05a** in the existing range verifier: generation-first
anchor witness presence; interior anchor-number monotonicity, witness presence
iff advancing, and present/nonzero identical root on inheritance. This adds no
new context schema, storage reader, EVM or detached verifier. The incoming edge
of a mid-generation range is now checked by P03-A05b using the required,
hash-bound excluded-parent header. Root/time authority, witness contents, age
and full anchor verification remain separate acceptance requirements.

The latest [Alethia #248 candidate](https://github.com/taikoxyz/alethia-reth/tree/ad584325c6b1e15f3c163fb25e71c04a78b194e0)
now selects the 4-second Etna fee target, including its first block. This removes
the old candidate's observed 2-second discrepancy from the current worklist;
it is source evidence, not a successful Raiko2 adoption or guest execution.
The candidate also uses OP-reth `op-v2.7.0-1` and Alloy consensus `2.5.0`, whereas
this Raiko2 baseline uses its earlier OP-reth v2.6-aligned revision. Dependency
and API/SDK compatibility still need a bounded adoption check; no pin was changed.
Raiko2 #305 still has no newer human review than `5333d6d6` at this refresh.

## L1 authority boundary amendment (2026-11-20)

Protocol discussion: [taiko-mono #22423](https://github.com/taikoxyz/taiko-mono/pull/22423)
(draft; provisional representation, protocol-owner review pending).

The user directed that facts L1 can establish at proof verification belong in
L1-constructed public inputs. The host/provider reads them; the guest binds
the values it actually uses to the same statement L1 reconstructs. Host reads
alone are not authority. This amendment supersedes blanket requirements below
to re-prove L1-readable facts inside the guest, only once the equivalent L1
predicate and statement binding are established. It changes no QC policy.

At inspected protocol head `7cd9606af6a8c9290874bd1525d58008e24ebadc`, existing
statement fields already bind L1's domain, generation, retained origin,
configuration digest, reference hash and blob commitment. The contract does
not independently authenticate historical signing sets: its terminal snapshot
comes from calldata. The proposed signing-set/schedule hash, or smaller
checkpoint-authority variant, needs coordinated protocol review before guest
provenance checks can be removed. Checkpoint identity and activation timing
must cover every current/next set used by the full range, including recovery
and overlap; configuration hash is not a signing-set hash.

- [ ] **P03-L01 — Authority inventory:** map each context value to its L1
  source, guest use, public field and historical selection predicate. Do not
  add guest storage proofs for fields already constructed authoritatively by
  L1. Retain reference-header/hash and execution/data binding obligations.
- [ ] **P03-L02 — Protocol review:** propose the smallest authority commitment,
  its L1 derivation/history and exact statement schema. Review selected-set
  authority versus registry-checkpoint authority, costs, H+2, A-to-B-to-A,
  recovery, full-width domain and `p < c`. Representation is not frozen.
- [ ] **P03-L03 — Coordinated acceptance:** after owner review, update the
  normative relation, native client, L1 statement/verifier and both guest
  journals/aggregation consistently; settle redundant embedded witness rules
  before omitting their validation. Include mismatch/replay/cutoff vectors.
- [ ] **P03-L04 — Narrow Raiko2 integration:** fetch in provider/host, bind in
  guest, reuse native signature/quorum and shared selection where still
  required. Remove the unneeded guest registry proof adapter only after its
  authority replacement is enforced, without changing existing EL/trie code.

Local registry witness work at `d535534f` and its later canonical-path
refinement are component experiments, not an adopted production path. They
remain unpushed and unwired. Pause further guest registry-proof integration
while P03-L01/L02 define its replacement. Do not present their native tests
as proof of an L1-authorized committee or complete Etna proving.

## Current state and source authority

Updated 2026-11-24. This plan is the authoritative progress tracker for
[Raiko2 #305](https://github.com/taikoxyz/raiko2/pull/305).
The user resumed implementation on 2026-11-19. Work now includes source
alignment, the bounded statement/cut codec, proof-specific execution projection
and per-height validator binding / required-QC verification through the shared
native checker. Review requests for other PRs are handled separately; only verified dependency or proof-rule changes
alter this plan. Keep shared build caches and resource limits in place.

| Source | Exact revision | Meaning |
| --- | --- | --- |
| Existing Raiko2 checkpoint | `845ecbcbe9c5cb81e1c09d0ca37464e58c2dbc18` | Implemented and tested consensus-input slice under the earlier design |
| Shared dependency | `4827fea0e844236d14cceafd6bd84a820bff2dd0` | Reviewed F31 child with only the shared raw-header extraction; no new QC behavior; not main |
| Patched SDK diagnostic | `a29b53c462b69035ef2acfb5785d78ba20067605` | Historical CPU guest feasibility with the then-current patch tables; diagnostics are no longer active workspace members |
| Merged design target | `93b59fe629dc8987ecb03c7d59d1f66004de068f` | taiko-mono #22412; reviewed head merged as `2e6763e2`, implementation adoption still separate |
| Shared body API candidate | `d7c6c3e30599463177152346d21dee6610f9f7f0` | [taiko-mono #22415](https://github.com/taikoxyz/taiko-mono/pull/22415), optional full-original-body helper; ready for external review; not the current Raiko2 dependency |
| Reviewed design candidate | `3fe7350119f2efe38cf588c145a9913bf9b3b398` | [taiko-mono #22419](https://github.com/taikoxyz/taiko-mono/pull/22419); approved after the import/gossip specification fix; proving clarifications recorded separately, no blanket source/layout adoption |
| Earlier client / protocol audit | `af1b952adb2d14263da60c5090e15e65b5d8d48c` / `4c6424bc97b226d100cc9ce9dc828c9567acef4e` | Historical 9-field input, 23-word statement and 4-word checkpoint snapshots |
| Later protocol / client witness audit | `7cd9606af6a8c9290874bd1525d58008e24ebadc` / `69090335f64df4ac509859a465f55f93d2f4c5d9` | Protocol source has 8-field input, 21-word statement and 3-word checkpoint; client account-witness schema inspected, generated settlement/backend agreement still pending |
| Inspected execution candidate | `53f33b708d869cfb3f8cccd597381fa2e92a5be5` | Anchorless execution exists; adoption also requires the OP-reth 2.7 / traits 0.8 / Alloy consensus 2.5 interoperability update |

The recovery-audit issue was fixed and the exact revised head was
[approved](https://github.com/taikoxyz/taiko-mono/pull/22412#pullrequestreview-5968429349), then merged at that same head.
That delta changes no proving rule or ABI. The earlier contract/client audit
used the superseded relation; the later protocol source above now has the
8/21/3 layout. Generated native-client and backend-journal agreement remains
open. The codec follows the normative proposed layout and independent ABI
vectors; this alone does not establish end-to-end implementation compatibility. No runtime,
SDK or execution dependency pin changes in this slice. Inspect later source deltas
before adoption rather than silently following development branches.

The #22419 review initially found an import/evidence-gossip dependency at
`d32f4568`: an empty block evidence budget does not disable historical lookups.
The revised `3fe73501` retains history while either stock age limit is met,
authenticates the excluded boundary and requires stock header/commit/validator
lookups, including the terminal block-commit key.
It was [re-reviewed and approved](https://github.com/taikoxyz/taiko-mono/pull/22419#pullrequestreview-5986783866).
Actual importer-to-peer acceptance remains an implementation gate. This is
node/import work, not a reason to duplicate EL/MPT logic in Raiko2. Its proof
changes clarify signing-set identity (keys and assigned powers) and permit one
narrowly scoped settled-switch root reuse; the latest fix changes no proving
rule. Candidate interface/storage changes and all anchor tasks remain gated.

## Devnet delivery phases (2026-11-20)

The user explicitly authorized FI to be disabled for the first devnet phase.
Phase one targets the complete Etna execution/Comet/committee/DA/statement path
in that configured environment. Phase two adds FI consumption, eligibility and
singleton-trial verification. This is a delivery split, not a claim that the
unrestricted normative proof relation is already implemented.

- Keep the same envelope witness positions, origin/cut/statement FI cursor
  fields and existing execution/prover ownership in both phases. Carry the
  authenticated cursor unchanged in phase one; do not remove it from the ABI.
- The phase-one deployment must actually disable FI with no unprocessed entries;
  the eventual guest composition must be restricted to that deployment/profile
  and reject FI witnesses or consumption. A host-controlled skip flag is not
  part of the proof interface. The exact profile binding is still P01/P06 work,
  not an implemented flag or a claim about existing devnet configuration.
- Phase two fills the existing FI verification seam with authenticated queue
  entries, due checks, consumed-cursor updates and the necessary trial. It must
  retain the phase-one range/statement/input architecture and existing EVM/MPT.
- P04's FI items and FI-specific P07 worst-case trials move to phase two. P03,
  non-FI execution and DA/range/output binding remain mandatory. The later SGX-
  first amendment selects one real SGX path for the initial internal testnet;
  both ZK guests and their proof/ELF/VK gates move to the subsequent ZK stage.
- The fewer-QC design challenge is open in [#305](https://github.com/taikoxyz/raiko2/pull/305#issuecomment-7030686273).
  Do not silently alter QC policy or claim protocol consensus while proceeding
  with independent byte/identity composition.

### Immediate range projection slice

Reuse the existing per-block projection to derive every intermediate Comet/EL
parent from the preceding matched input over a complete bounded `(p,h]`, then
bind both terminal hashes to the expected statement endpoints. This supplies
the missing Etna Comet/envelope binding around existing EL range replay; it does
not reimplement the EL validator, aggregation framework or BFT signing process.
Comet links compare the predecessor header hash under the current target; no
stock full-BlockID or embedded-LastCommit rule is added. Origin/schedule
authentication, nested witnesses, execution, anchor/time rules, DA and final
statement composition remain separate gates.

## Parallel review findings and merge gates

The first external review at 0c218cbd/aeda3262 is tracked alongside implementation.
The source inspection confirmed #22398 merged into feat/taiko-client-rs-abci,
not taiko-mono main. The immutable F31-based development pin (now the narrow
4827fea0 codec carrier) is intentional for this draft. Before merging into Raiko2 main, use an appropriate reviewed upstream
main/tag revision or explicitly resolve the source-lifetime/build-reproducibility
gate with reviewers; do not call feature-branch integration mainline adoption.
The obsolete e6 diagnostic pin is removed with its executable diagnostic targets.

- [x] R01: deprecate the superseded public certified-envelope gate; retain seven clearly historical controls.
- [x] R02: expose original committed transaction payload spans; cover both generation-parent guards and uint48 overflow; do not trust cached hashes or recovered signers.
- [x] R03: add checked statement range/interior-cut count validation before hashing while preserving raw ABI vectors.
- [x] R04: cover all manifest dependency groups and every taiko-mono package in pin guards.
- [x] R05: remove diagnostic production-workspace coupling and preserve commit-pinned historical evidence.
- [ ] R06: settle upstream dependency provenance before main merge.
- [ ] R07: run `just build-guest all --force` at the final source, commit generated provenance and pass the unskipped artifact/schedule-export CI gates before main merge.
- [x] R08: lint Etna integration tests explicitly in CI with all targets.

The scoped review fixes passed 68 protocol tests (7 historical controls), 44
xtask library tests, strict all-target Clippy for both touched crates, workspace
formatting and an isolated no_std consumer/normal-feature audit. Three projection
guard mutations each compiled and failed its exact new regression; the unmutated
source was compiled again and all 68 tests passed. One initial mutation run reused
a stale artifact at the shared virtual path; that result was discarded. Subsequent
variants forced source freshness without another target directory. Independent
source review and independent behavioral verification passed. Workflow YAML was
parsed with the Ruby standard library and its scoped Etna all-target lane checked.
These checks do not replace final-head production rebuilds or upstream mainline
source availability. R06/R07 remain unchecked and #305 remains draft.

## Milestone progress

`DONE (earlier baseline)` records reusable evidence, not conformance to #22412.
`PARTIAL` means the listed code exists but its remaining acceptance gate is open.
`TODO` means no completion claim. Dependencies are deliverable gates, not a
requirement to run independent inspections serially. No percentage is inferred
from checkbox counts.

| ID | Deliverable | Status | Depends on | Completion gate / evidence |
| --- | --- | --- | --- | --- |
| P00 | Portable shared core and patched SDK feasibility | DONE (earlier baseline) | — | Merged #22398; diagnostic README and checkpoint `a29b53c4` |
| P01 | Freeze revised spec, implementation pins and ABI vectors | PARTIAL | Compatible upstream implementations | Design approval, source matrix and codec vectors recorded; native ABI/backend framing agreement pending |
| P02 | Canonical supplied-block input and EL/Comet binding | PARTIAL: complete range/parent projection | P01 for revised behavior | Bounded original-byte projection and Comet/EL chains bind the expected endpoints; native DA/SSZ/execution composition is tested; strict context witnesses and authority remain open; #22415 is optional |
| P03 | Authenticated origin, L1 context and committee schedule | PARTIAL: native uniform SSZ/anchor-root binding | P01, P02 | Reference/anchor history and structural schedule checks complete as components; origin/profile/Inbox authority, time/rules, committee derivation and normative SSZ/recovery coordination remain open |
| P04 | Strict Etna execution; FI in phase two | PARTIAL: native strict supplied-block replay complete | P01, P02 | B04 native execution/DA composition tested with genuine EVM/MPT; canonical-main source adoption, complete authenticated context and actual SGX consumer remain; FI phase two stays held |
| P05 | Range consensus coverage, DA, cuts and statement | PARTIAL: native authenticated DA/range/statement composition | P02–P04 for full range | Matched Comet/EL identities and supplied cursor shape bind every interior cut and statement endpoint; native strict execution is joined; actual FI/context authority and final guest composition remain pending |
| P06 | Provider, pipeline and backend wiring | PARTIAL: owned native input and existing-verifier adapter | P02–P05 | Serialization/replay component tested; actual collection, task identities and SGX/provider/pipeline consumption of the complete authenticated input remain |
| P07 | Both real guests, aggregation and proof acceptance | TODO | P05, P06 | Real proofs, child continuity, permitted L1 verifier combination and resource bounds |
| P08 | Regression, independent final review and ready handoff | TODO | P01–P07 | Required checks, Unzen regressions and exact-head review; no unresolved material findings |


## Reuse audit and scope correction (2026-11-20)

- Removed the unused independent Etna L1 verifier and its 159-vector scaffold; reuse existing L1 linkage at its actual caller and retain the raw/future-header adapter requirement as an open integration gap.
- Removed ordinary transaction-root recomputation from the Comet/EL byte projection. Existing `stateless::validate_block` consensus/pre-execution remains authoritative for the same matched block; its existing `rejects_mismatched_transaction_root` test is the regression check.
- Retained BFT/domain/committee duties, original envelope/EL byte equality, cross-layer Comet/EL height, generation and independently authenticated expected-parent binding. Generic EL validation against an arbitrary supplied parent is not a substitute for that last relation.
- No account/MPT implementation was added. Reuse existing `SparseState` / `CachedTrie`; future work is Etna-specific root/address/slot/context composition.
- Restored the earlier near/far L1-reference decision in the paired design; its historical SSZ bridge and current normative linear-chain rules still need explicit reconciliation.

The reuse correction passed the then-current 25-test fork crate plus the common
EL rejection regression, scoped Clippy/format/no_std checks and independent
review/verification. The BFT and review-fix slices below expand the fork crate to 68 tests.
The common regression exercises the existing Shasta/Unzen validator; Etna fork
wiring is still open and is not established by that check.
Earlier standalone L1 evidence is withdrawn from the completion gates. No source
or SDK pin, EVM/trie/provider/pipeline or production guest/ELF change is included.

## What #22412 changes in the proving plan

| Previous assumption | Revised target | Work affected |
| --- | --- | --- |
| Guest replays ValState and proposer priorities | Priorities are node/Comet state; the guest does not check `app_hash` or its ValState preimage | P02, P03, P05; remove priority arithmetic from the planned guest, keep committee derivation |
| Comet `app_hash` equals the parent EL hash | Node `app_hash` commits EL hash plus ValState; guest binds EL through the canonical envelope and EL parent chain | P02; the existing core body helper's bare-EL equality cannot be reused unchanged |
| Every block and embedded LastCommit needs QC verification | Verify h and the last height of each signing-committee interval in `(p,h]`; bind all other headers by ancestry | P02, P05, P07; derive coverage, do not accept a host-provided skip flag |
| Grid-aligned range/cuts, 23-word statement, 9-field input | Any terminal height; every interior cut; 21 words / 672 bytes; 8-field LandInput | P01, P05–P07; 4-word cut leaf and 3-word checkpoint record |
| Minimum proof unit is K blocks | Every valid block must be provable alone, including any required FI trial | P04, P07; singleton and worst-case proof acceptance |
| Registry displacement and old evidence limits | Lowest empty slot; source-defined supply/minStake constraint; 900 blocks / 1 hour evidence limits | P01, P03; native/fixture conformance, not guest evidence-pool replay |

The existing strict gate is evidence about the previous relation. Its primitive
codecs, signatures, body commitments, bounds and fixture oracles are reusable;
its bare-EL app-hash check and mandatory predecessor QC are not the final new
range verifier. Retaining the old tests as historical controls does not make
them the acceptance definition for the new relation.

## Global constraints and review focus

- Review #305 in parallel with implementation. Before each bounded step and
  before submitting its checkpoint, refresh the exact head, review replies,
  inline comments and CI. Handle material interface/proof-boundary feedback before
  building dependent changes; preserve reviewed commits and keep deltas small.
  At the `0c218cbd` checkpoint there were no submitted reviews or comments.

- Keep Raiko2's EVM, CachedTrie, witness and execution DB ownership. No client
  service migration, new generic Protocol trait, trie replacement or mining replay.
- Pin reviewed revisions. Regenerate ABI/bindings through their existing tools;
  do not hand-edit generated contracts, ELFs, VKs or provenance.
- Committee provenance, keys/powers, strict weighted quorum, L1 roots, EL/FI/DA
  and complete range/output binding remain proven. Removing priorities does not
  authorize an arbitrary host committee or skipped execution.
- Do not put a ValState hash back into settlement to compensate for an old API.
  App-hash/priority replay and stock evidence-pool behavior are client concerns.
- Derive mandatory QC targets from the authenticated schedule and range. Extra
  child-proof boundaries must not silently require otherwise unnecessary QCs.
- Preserve `(p,h]` when settlement advances to c: for `p <= c < h`, retain the
  original proof origin, blobs, cuts and statement; do not rewrite into `(c,h]`.
- Never turn missing witness, timeout or resource failure into a valid FI skip.
  A committed list is executed completely, including ordinary revert/OOG.
- No Unzen/Etna or recovery-generation-spanning proof. Preserve stop/settle/restart
  at the special fork and generation-qualified identities thereafter.
- Reuse the configured shared target and existing crypto patches, serialize builds,
  bound jobs/disk use and freeze source before expensive guest builds. No new SDK,
  duplicate target, cleanup or benchmark campaign without task-specific need.
- For each implemented milestone: focused RED/GREEN controls, scoped checks,
  independent review, and independent behavior verification for proof/consensus.
  Record source/command/results; fix and recheck material findings before DONE.

## P00 — Shared-core and SDK feasibility: completed evidence

- [x] Deliver portable committee/key/power/schedule, native Comet codecs and QC/body primitives upstream (#22398).
- [x] Run the stock diagnostic in actual patched RISC0 3.0.5 and SP1 6.8.1 CPU guests; compare native and independent oracle journals.
- [x] Independently review the previous Raiko2 consensus gate: seven authored tests, six independent groups, focused strict Clippy and workspace formatting.

Evidence: [historical diagnostic results](../../research/2026-11-18-etna-shared-core-guest-compatibility.md),
[gate tests](../../../crates/protocol-etna/tests/certified_envelope.rs), and
[#305](https://github.com/taikoxyz/raiko2/pull/305). These are not real proofs of
#22412. The diagnostic workspace/lock additions were later removed. Root manifest and build-tool changes still require production provenance refresh.

## P01 — Freeze the new relation and source interfaces

**Owns:** dependency/source matrix, protocol codecs and shared-vector provenance;
root/guest manifests and generated binding entrypoints only when adopting code.

- [x] Record reviewed design `93b59fe6` and its exact-head approval; its recovery-only delta leaves the proving/ABI target unchanged. Later deltas still require inspection.
- [x] Compare actual client `af1b952a`, protocol `4c6424bc` and execution candidate `53f33b70` with the design: old 9/23/4 settlement and bare-EL app hash remain; execution adoption changes the Reth/Alloy graph.
- [ ] Pin a compatible reviewed implementation set, including execution fork and rules/SDK identities. Do not treat the preceding source audit as completed integration.
- [x] Implement pure typed ABI encoders for the proposed 8-field input, 21-word / 672-byte statement preimage and 32-byte Keccak digest, four-word cut leaf and five-word cutHash. Cross-check representative and full-width codec-only vectors against independent manual packing and offline Cast.
- [ ] Compare generated native contract/client layouts, selectors and vectors, including the three-word record and semantic uint48/range boundaries. Record the exact public-input/journal framing required by each pinned backend verifier.
- [x] Specify and independently verify the structural-only shared body API candidate in [taiko-mono #22415](https://github.com/taikoxyz/taiko-mono/pull/22415), without AppHash or certificate policy and without native caller changes.
- [ ] Adopt additional shared APIs only when a concrete caller benefits. #22415 is optional, not a global gate; the direct proof-specific path uses existing F31 header/DataHash primitives. Keep the current pin while its review proceeds.

Evidence for the encoding subtask: [source](../../../crates/protocol-etna/src/statement.rs),
[golden tests](../../../crates/protocol-etna/tests/land_statement.rs) and
[fixture provenance](../../../crates/protocol-etna/tests/fixtures/README.md).
Thirteen crate tests (seven earlier-gate plus six codec) and two independent
oracle cases passed; scoped strict Clippy and workspace formatting passed.
The codec deliberately does not enforce authentication, range/count/budget
constraints or semantic uint48 bounds. Full-width vectors test encoding only.

**Accept:** cross-language preimage and digest golden vectors agree with the
matching native contract/client encoders and backend framing; old 23-word,
missing-field, reordered-cut and wrong-generation inputs reject. The matrix
identifies implemented versus specified behavior. Historical pins alone do not close P01.

Build consistency was repaired without changing runtime pins or guest locks. The test
now distinguishes independent packages in the same repository and follows the actual
`guest-common` dependency edges to the precise locked commits. The full xtask suite
with `digests` passed 42 tests; two independent controls, strict Clippy and formatting
passed. The existing CI lanes now include `protocol-etna`; the Python CI environment
also declares its missing Keccak backend. Production guest provenance remains an
open draft gate, rather than being rewritten to match stale artifacts. Unused
private signing fields were removed from the diagnostic generator inputs; all
public values and the pinned 684-byte journal are unchanged. Native re-execution
passed; historical guest ELF/cycle measurements were not repeated.

## P02 — Supplied blocks and canonical envelope binding

**Files:** `crates/protocol-etna/src/lib.rs`, narrow envelope/input modules in that
crate; `crates/protocol-etna/tests/certified_envelope.rs` and new envelope tests.

- [x] Build the earlier canonical-wire/current-QC/body/embedded-QC gate in `845ecbcb`; keep its evidence scoped to that relation.
- [x] Implement and independently verify the narrow upstream body API candidate in #22415: 67 core tests and three independent checks, including 26 Go vectors and 2,112 differential inputs. It recomputes full BlockID and returns exact transaction bytes; it authenticates no identity, domain, version, committee or execution by itself. External review and Raiko2 adoption remain open.
- [x] Bind the exact v6 envelope shell and supplied EL projection with existing canonical RLP and Ethereum codecs. Preserve byte identity and the normative/native wire schema; no ABCI/lander service import. Full strict nested-witness decoding/authentication remains required in P03/P04.
- [ ] Bind every supplied EL header/body to its committed envelope, height, parent and generation; treat Comet app-hash bytes as opaque to the proof, as the revised spec requires.
- [ ] Compose header-hash ancestry, DataHash binding and required endpoint certificates. Recompute full original part sets only for callers that require that stronger body relation; do not add it to every guest block by default. All header identities still require authentication.

### Immediate P02 projection slice

Use the current F31 `header_hash`, `data_hash`, domain and app-version primitives.
`projection.rs` binds a caller-authenticated expected Comet header hash and the
single exact envelope to the actual supplied Ethereum Header and typed
transactions. It checks domain/version/height, fixed-origin header ancestry,
EL parent and 13-byte generation/anchor extraData; no AppHash-content check or
per-height detached QC is introduced.

The v6 parser checks only the strict outer shell and optional slot arity, retaining
borrowed execution/witness RLP spans. Compare the Ethereum Header's canonical
encode/decode fixed point and ordered EIP-2718 byte strings;
limit the complete original execution item to 124,000 bytes. Native wire vectors
and real public signed transaction fixtures guard compatibility. The returned
projection is not complete envelope or proof validation: nested witness grammar,
authenticated provenance/schedules, all-height validator hashes, strict execution,
FI, DA and endpoint QCs stay mandatory in their composing milestones.

- [x] Add behavioral RED and implement the bounded projection in the existing fork crate.
- [x] Preserve all external dependency records; inspect normal no_std features.
- [x] Run focused/independent native wire and mutation tests, strict scoped Clippy and formatting; obtain independent adversarial review.

Historical projection evidence at `39f803b5`: [implementation](https://github.com/taikoxyz/raiko2/blob/39f803b532df88f688a350ef95bcf8adce0021f5/crates/protocol-etna/src/projection.rs)
and [12 regression tests](https://github.com/taikoxyz/raiko2/blob/39f803b532df88f688a350ef95bcf8adce0021f5/crates/protocol-etna/tests/execution_projection.rs).
The semantic RED failed at the unimplemented verifier, then the final native
`cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`
run passed 33 tests: 12 projection, seven earlier-gate, six statement and eight
independent scratch-only native-oracle groups. The latter used the exact
`envelope.rs` and raw-L1-header codec from client `af1b952a`, compiled separately
against its locked Alloy consensus 2.5.0, then consumed those bytes with Raiko2's
unchanged 2.4.2. Public signed legacy/EIP-1559 inputs, optional witnesses,
124000/124001-byte execution items and a larger witness-only envelope were covered.
The native encoder source SHA256 was
`4171e5e1cd72fe46853047fcc21676277c73946318d37cee49c255a3da0cc26e`.
These are wire fixtures, not valid complete Etna blocks or state-transition proofs.

Strict scoped Clippy and workspace formatting passed. A separate locked
`#![no_std]` consumer compiled, and its normal dependency tree activated no `std`,
`use_std` or `use-std` feature. All 1207 other root lock package records remain
unchanged; only three direct existing-version Alloy edges were added to the fork
crate. Independent adversarial review and independent behavioral verification
passed. Validation used host Rust 1.95; no production guest build, ELF refresh,
real proof, cycle/performance measurement or end-to-end integration is claimed.

A scratch-only patch of #22415 into the current Raiko2 Rust 1.95 graph passed
14 native tests: the new API consumer plus all seven earlier gate and six
statement cases. The consumer matches the Go header/part hashes and current
Tendermint type, including opaque AppHash bytes. The other 1,207 lock records
are unchanged. This verifies native crate compatibility, not canonical envelope
binding, guest/proof acceptance or dependency adoption. The real checkout
keeps the merged `f31e7ec5` pin.

**Accept:** valid new app hashes and alternate permitted beneficiaries pass;
substituted envelope/EL bytes, raw/projection mismatch, height/domain/parent errors,
malformed framing and changed authenticated header commitments reject. Full-BlockID
callers separately reject changed original part sets. No full-transition claim
is attached to an unauthenticated input or a structural-only result.

### Complete range projection checkpoint (2026-11-20)

`range::verify_range_projection` is a thin caller of the existing per-block
projection. It checks `origin <= p < h < 2^48`, representable/budgeted exact
input count before processing, derives each Comet/EL parent from the preceding
matched block and binds both terminal hashes to the expected endpoints. A
generation's first Comet header has no parent ID; later ranges bind the first
header to p. The result retains original execution/transaction/anchor/committee/
FI spans and inherits the current APP_VERSION policy. It is byte/identity
projection, not a certified range or state-transition proof.

The valid three-block case compiled and failed at the RED stub. Frozen GREEN
passed all **86 protocol tests** (68 prior, including 7 historical controls,
plus 18 range cases), strict package all-target Clippy and workspace formatting.
Five separately authored scratch tests compiled and passed against the same
production source. Forged first/interior Comet and EL parents remain rejected
after resealing descendants and terminal identities; singleton/later-origin,
uint48, count/byte bounds, generation, original spans and each endpoint have
direct controls. The isolated no_std consumer compiles and its 221 normal
feature rows enable no std/use_std/use-std. Independent adversarial source
review and behavioral verification passed.

One nonblocking cost observation remains: the current public-helper composition
hashes each Comet header twice. It does not change acceptance; account for it
when measuring/wiring the real guest rather than widening this slice into an
upstream API refactor. No guest cycle measurement or proof build was run.
Source/runtime pins, manifests/locks, EL/trie, provider/pipeline, production
guests and ELF/provenance are unchanged. Origin/storage/committee provenance,
strict nested witnesses, execution/time/anchor checks, DA/cuts and final backend
composition still gate P02/P03/P05–P08. Phase-one FI restrictions are not yet a
guest implementation. No QC policy conclusion is implied by these tests.

## P03 — Authenticated context and committees

**Files:** narrow context modules in `crates/protocol-etna`, the existing
witness-backed read APIs in `crates/stateless`, and Etna guest composition.

- [ ] Authenticate proof origin, generation, parent EL/Comet identities, L1 reference/header ancestry, fixed parameters, applicable rules and planned stop.
- [ ] On the switching boundary, verify the L2 SignalService proxy account/EIP-1967 slot under S's settled state root against the implementation built into the guest (PROOF-6.2-46a). The originWriter timestamp precheck and node-local `beaconRootsFrom`/fork-time trust check remain originWriter/client responsibilities, not additional guest witnesses.
- [ ] Adapt existing authenticated storage reads to registry snapshots, complete entry enumeration, latest-at-cutoff selection, records and power derivation; derive active/pending committee timing including H+2 and recovery-origin floors.
- [ ] At every height, compare both `validators_hash` and `next_validators_hash` with the independently authenticated active/pending schedule (PROOF-6.2-18), including interior heights without a required QC. Keep proposer-priority arithmetic and ValState preimages outside this guest path.

**Accept:** wrong root/slot/count/cutoff/key/power/epoch/generation and substituted
origin reject; initial deployment, nonzero S, ordinary handover and recovery-origin
cases pass. Mutate current and next validator hashes at a non-QC endpoint while
resealing the authentic ancestry and endpoint QCs; both mutations must reject.
Reuse shared algorithms; do not accept host success flags.


### P03 reuse and near/far reference integration

The unused independent L1 module and its tests/vectors from `1bc5bea6` were
withdrawn after the reuse audit. Its earlier native checks remain historical
evidence in that commit, not completed integration. Generic hash linkage already
exists in `guest-common`; account/storage authentication already exists in
`stateless` over `SparseState` / `CachedTrie`. Do not add another generic verifier.
Existing public helpers are parent-state-oriented; add only a thin expected-L1-root
adapter when a real Etna call site needs it. Preserve exact witness address/slot
and format obligations. Typed L1 headers cannot preserve unknown future fields:
track the raw-header adapter gap when reusing linkage rather than treating it as
already solved or restoring a parallel library in advance of its consumer.

The [paired design's recovered near/far strategy](../specs/2026-11-17-etna-shared-proving-design.md#recovered-long-distance-anchor-decision)
restores the prior N..L / Q policy and links the original design and research.
Reuse generic linkage; keep Etna reference selection separate from Shasta origin
metadata. Normal Q=L needs no submission-delay tail when its expected landing
fits the BLOCKHASH/EIP-2935 query window. Otherwise choose newer Q and use a short
tail or beacon `block_roots` / `historical_summaries` bridge for L, retaining N..L.
All other required L1 roots must still be independently covered. Account for
proving/inclusion headroom, raw future fields, fork/slot/path bindings and archive
availability. Execution-block lookup age is distinct from beacon-slot windows.

The anchor work is an explicit tracked backlog; adding these items does not
mark their implementation or upstream specification alignment complete.

- [ ] **P03-A01 — Range/root inventory:** derive N and L from the actual covered blocks, cross-check requested latest anchor L, include every required origin/committee/rules root and bind selected Q's number/hash to the same job and landing statement.
- [ ] **P03-A02 — Near reference/headroom:** prefer Q=L when it covers the inventory and remains queryable through proving, queueing, retries and inclusion. Cover BLOCKHASH ages 1/256, EIP-2935 ages 257/8191 and expiry at 8192 execution blocks; prefer finalized Q where the query window allows.
- [x] **P03-A03 — Header/link reuse (native A08 composition):** wire the existing generic L1 linkage at the real Etna caller with only the raw future-field adapter, count/byte bounds and expected-root binding it needs. Do not restore the unused independent L1 verifier or a new trie/framework.
- [ ] **P03-A04 — Distant bridge:** retain N..L, then use a short tail or one authenticated Ethereum beacon `block_roots` / `historical_summaries` bridge from L to newer Q. Bind Q's fork-specific beacon-root field from the same raw header, fork/slot/index/list length and original hash of L. Do not confuse 8191 execution blocks with 8192 beacon slots or L1 beacon roots with L2 anchor state roots. Test missed slots, period boundaries, expired reference and missing archive data.
- [ ] **P03-A05 — Anchor/root semantics:** reuse existing MPT reads at the authenticated L1 root, check Etna witness presence/inheritance, generation/age/progression, expected addresses/slots and EL anchor-number/parentBeaconBlockRoot equality. Every new or inherited anchor and recovery snapshot keeps its required authentication.
- [x] **P03-A05a — Structural range transitions:** enforce the generation-first and covered-interior witness/number/inherited-root relation at the existing range caller; authenticate actual L1 root/time, origin floor and the incoming mid-range edge separately.
- [ ] **P03-A06 — Specification gate:** reconcile the retained historical bridge policy with normative PROOF-6.2-33/34 and BATCH-6.3-5..7 before enabling it; current target still describes a linear resource/recovery limit. The candidate's settled-switch exception is not a general historical-anchor bypass.
- [ ] **P03-A07 — Settled-switch candidate:** after source/spec adoption, authenticate an ordinary S(t) with generationStartHeight < S(t) <= p through consecutive same-generation EL hashes/heights back from p's authenticated slot-285 EL hash. Use S(t)'s root only for that rotation's registry witness, still derive D(t)/origin-floor selection and H+2 activation, and reject old-generation/discarded headers. This #22419 candidate does not exempt covered block anchors or recovery R from L1 ancestry.

No historical SSZ guest, archive provider, reference-selection host API or full
origin/committee proof is claimed implemented by this audit.

### P03-A05c — Shared raw-header prerequisite (2026-11-22)

[taiko-mono #22426](https://github.com/taikoxyz/taiko-mono/pull/22426), draft at
`8268eb63d71a6a1602880722f44550eb43bea05b`, extracts the existing ABCI raw L1
header parser into `etna_core::l1_header::L1HeaderView`. Its source base is the
client branch `fbcec295`, not mono main. ABCI delegates to the same parser while
retaining its owned header/wire API. The only manifest/lock change is an existing
local ABCI-to-core edge; no third-party source or version changes. Raiko2 remains
pinned to F31 at that checkpoint. A08 now adopts the exact extraction on its
own F31 child for the real native caller, without the unrelated source delta.

- [x] **P03-A05c1 — Portable shared codec candidate:** exact borrowed raw-header
  identity and four positional fields, original 4096-byte limit, error ordering
  and opaque direct-item behavior. The new API has an actual ABCI caller; it
  does not restore the unused standalone L1 verifier.
- [ ] **P03-A05c2 — Review/source gate:** complete upstream review and full native
  client CI before merging the draft; select a compatible reviewed pin before
  downstream use. Recheck the F31-to-selected-source delta rather than upgrading
  unrelated QC/committee semantics during their hold. R06 remains open.
- [ ] **P03-A05c3 — Anchor/reference/time composition:** native raw-header/root
  linkage is complete under A08; remaining time/age and Inbox facts are pending.
  The combined requirement is to open the exact carried
  header bytes against the selected authenticated near-reference ancestry,
  compare anchor number/root to the same matched EL header and roll inherited
  facts through the range. Only when p+1 inherits does it need the excluded
  parent's raw anchor opening (PROOF-6.2-47). Then check the exact timestamp
  maximum and authenticated age/origin limits, rejecting parent-time overflow.
  Reuse the existing proof engine for required storage predicates; do not add
  another trie or treat a host read as authenticated state.

Reuse refinement: Shasta's L1 linkage lives inside a private event-bound helper
with its own stalled-anchor policy; it is not a directly callable Etna verifier.
Keep that policy scoped to Shasta and factor only useful low-level link checks
when the Etna caller is present. No generic framework or new storage reader.

Candidate evidence: 58 core unit tests and 27 exact-source adapter/oracle tests
(11 delegated, 11 pinned original and five independent manual-fixture cases)
passed offline against upstream-locked dependency identities. Strict core doc
lint and all-target core/adapter Clippy passed. A separate no_std consumer passed,
and its 138 normal runtime feature rows excluding host proc-macros have no std.
The pinned formatting recipe and real native workspace metadata passed. Full
`just verify`/native-client/Docker checks remain open on the upstream draft;
this harness is not their substitute. No Raiko2 guest/ELF, dependency or runtime
code change is included in this checkpoint.

### P03-A05b — Excluded-parent binding (2026-11-22)

Continue A05a at the existing range and statement functions with the actual
excluded-parent EL header as a required native argument. Bind its number and
canonical hash to the same existing range context. Reuse the covered-header
round-trip check to keep the typed parent fields faithful to those same encoded
bytes; a hash over an ambiguous optional-field representation is insufficient
as a generic field-binding contract. This is header identity/codec verification,
not re-execution or a new consensus engine.

For a mid-generation origin, initialize the existing anchor-transition checks
from the parent's same-generation Etna extraData and anchor root. For the fixed
generation origin, bind identity but ignore its old anchor/extraData: genesis,
Unzen and prior-generation recovery parents must remain supported, with a carried
first-block witness. Do not add an optional parent or a skip-compatible path.
The parent is already needed for execution; this freezes no final guest-input,
ABI, committee or public-input schema and introduces no storage reads.

- [x] **P03-A05b:** verify the required parent's height/hash/field fidelity and
  the incoming mid-generation anchor edge; propagate failures through the same
  statement wrapper and retain genesis/recovery exceptions. Add alias/parent
  mutation and inherited/advancing/regressing boundary controls.

This closes A05a's intentionally open incoming edge at the native range boundary.
L1 witness/root/time authority, origin L1 floor, checkpoint/rules/age checks,
strict DA and the complete execution/proof path remain separate requirements.

A05b acceptance evidence:

- Compiled semantic RED: 37 existing/positive range controls passed and nine
  negative controls failed because the old loop ignored the supplied parent.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`:
  120 maintained tests plus ten independently authored scratch tests passed,
  including 46 maintained range tests. Parent mutations preserve or rebind the
  intended hash as appropriate; descendant chains and endpoints are resealed.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings`,
  `cargo fmt --all -- --check` and the locked standalone no_std consumer passed.
  The normal runtime feature audit has 221 rows and no std/use_std/use-std.
- Independent adversarial source review and independent behavioral verification
  passed at the same final production/test bytes. Same-RLP/hash optional-field
  aliases, same-hash wrong height, root/extraData mutation, full generation and
  fee/length checks, incoming witness/root transitions and uint48 boundaries
  reject correctly; canonical shorter origin headers remain supported.
- Two test-only lint fixes changed no production bytes or assertions. Final
  checks used immutable offline snapshots and the existing shared target with
  a 15 GiB reserve. No dependency, guest artifact or backend rebuild was run.

The required native parameter is updated at every actual caller. No optional
compatibility route or host bypass is retained. This is not a finalized guest
transport schema or complete L1/anchor/proof authentication.

### P03-A05a acceptance evidence (2026-11-22)

The existing `verify_range_projection` loop now checks generation-first witness
presence and covered adjacent anchor transitions. Its statement wrapper receives
the same rejection behavior without another API or parser. The per-block checker
still returns original borrowed witness bytes; this is not witness authentication.
For `p > origin`, bind p's actual EL header before claiming the incoming transition
is verified. Future composition must also authenticate anchor roots/time, origin
floor, checkpoints, rules, age and recovery state.

- Semantic RED: the 27 original range tests and two new positive controls passed;
  six negative tests failed because the old loop accepted the invalid transitions.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`:
  109 maintained tests plus five independently authored scratch tests passed,
  including 35 maintained range tests. Mutations reseal DataHash, both ancestry
  chains and endpoints, so rejection is attributable to the new transition check.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings`,
  `cargo fmt --all -- --check`, and the locked standalone no_std consumer passed;
  its normal runtime feature audit has 221 rows with no std/use_std/use-std.
- Independent adversarial source review and independent behavioral verification
  passed. Generation-origin singleton, recovery-origin, mid-generation first-edge
  boundary, witness omission/redundancy, uint48 anchor values and changed/missing/
  zero inherited roots have behavioral controls. Tests use immutable offline
  snapshots and the existing shared target with a 15 GiB reserve.

No source pin, manifest, lock, EVM/trie, guest entrypoint, production ELF/VK or
final ABI changed. R06/R07 and the complete proving path remain open. The current
head's xtask failure was read back as the existing RISC0 source-fingerprint gate,
not a failing native protocol test.

### Immediate BFT schedule binding / QC requirements

Implement the P03/P05 field relation using existing `Validators.hash()` and
caller-authenticated active/next sets at each height, with complete `(p,h]`
coverage and no host skip flags. Check both hashes even at non-QC heights and
check next/active coherence across neighbors. Derive h plus the outgoing endpoint
of every change in signing keys or assigned powers; metadata-only rotation adds
no target and aggregation child boundaries do not create protocol QCs.

- [x] Add compiled semantic RED, the minimal field-binding/target derivation and meaningful full-range/mutation boundaries.
- [x] Run focused and independent exhaustive-model tests, strict scoped Clippy, formatting and standalone no_std checks; obtain independent source review.
- [x] Verify an exact ordered commit list at all derived targets through the shared native QC checker, using the corresponding checked header/active set and the canonical deployment domain.
- [ ] Compose these helpers with independently derived schedules, authenticated ancestry and the final adopted QC policy. Matching supplied schedules and native certificates alone do not establish schedule authority or full proof validity.

Implemented in `crates/protocol-etna/src/coverage.rs`. For independently
supplied active/next sets and authenticated full-landing endpoints, the helper
checks exact counts, consecutive uint48 heights, both commitments at every height
and next/active coherence. It reuses the shared canonical cached keys/powers
hash. The result is a requirement list, not a certified range or verified QC.

Acceptance evidence on host Rust 1.95:

- The valid A,A,B,B,A range compiled and failed against the stub before GREEN;
  the implemented relation returns the outgoing endpoints plus h.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`: 68 passed, including 7 explicitly historical gate controls, 11 schedule cases, 5 model cases, 17 projection cases, 12 statement cases, 13 signed native-QC cases and 3 independent certificate cases.
- The independent model checks 1,360 short schedules using explicit sorted keys/powers, 10,016 current/next hash substitutions and 3,648 disconnected adjacent schedule mutations. It is finite regression coverage, not an unbounded theorem or composer test.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings` and `cargo fmt --all -- --check` passed. A separate locked no-default-features `#![no_std]` consumer compiled; its 221-record normal-feature closure enabled no `std`, `use_std` or `use-std` feature.
- Independent adversarial source review and independent behavioral verification passed. Immutable offline source, the existing shared target/lock, two jobs and a 15 GiB reserve were used. The native-QC and review-fix slices keep the existing runtime pins, EL/MPT, provider/pipeline and production ELF bytes; the review fixes restore both guest manifest/lock pairs to base.

These checks do not authenticate the supplied schedule, derive origin/H+2 or
verify header ancestry, DA, execution, FI or a final proof. Native commit
signatures are checked only by the explicitly pinned native helper below.
Aggregation must pass the whole landing endpoints; a shorter child cut cannot
stand in for them. P03/P05 remain partial until the real composition is tested.

`verify_required_native_commits` first reuses the complete schedule binding,
requires exactly one commit at each derived height in ascending order and checks
all target heights before any native signature verification. It derives the
chain ID from the full `DeploymentDomain` and uses checked offsets to select the
same height's header and active set for `verify_native_commit`. It does not add
origin, embedded-LastCommit, aggregation-child or non-target certificates.
Callers must bound and canonically decode raw commit witnesses before this API.

The signed tests cover A-to-B-to-A, a missing outgoing commit, extra/duplicate/
reordered targets, a later malformed target before an invalid first QC, wrong
signed header or outgoing keys, full-width chain IDs/Inbox/generation, exact
2/3 rejection, nil/absent votes and malformed present slots. The independently
signed vote-domain mutation preserves the header/BlockID to isolate signature
domain binding. A re-signed APP_VERSION mutation proves the inherited restriction
is the rejecting predicate. The native positive RED first passed the shared QC
checker, then failed at the new stub before implementation. Two test-only lint
issues were fixed; the final frozen source passed all checks and independent
source/behavioral review.

P01 must also reconcile F31 `verify_native_commit`'s APP_VERSION restriction with
the candidate's explicit guest field table before using it as the complete new
range certificate relation. This coverage component changes no version policy
or legacy gate and does not silently solve that adoption question.

## P04 — Existing execution and FI verification

**Files:** `crates/stateless/src/validation.rs`, `crates/guest-common/src/etna.rs`
when introduced, and focused tests in their existing crates.

- [ ] Align the Etna EVM dependency and select fork-specific header/system-call behavior, including the source-defined extraData/generation/anchor fields, required fee-share value and nonzero anchor-root context. Remove legacy anchor/first-receipt-success assumptions only on the Etna path; preserve Unzen checks.
- [ ] Wire the entire exact committed Etna body through existing strict consensus/execution/witness/trie/post-root code; ordinary transaction-root, header/body and STF checks stay there. Reject `[valid, zk-overflow, valid]` rather than filtering it into a shorter block; accept valid empty bodies and ordinary first-tx revert/OOG.
- [ ] Authenticate the FI queue entry and bytes, due-time/eligibility rules and cursor progression. Complete any required singleton trial, or verify an equivalent rejection predicate against the same parent, attributes and full budgets.
- [ ] Retain reads required before a discard, including storage/code/BLOCKHASH/EIP-7702; isolate trial effects from actual execution. Missing data and infrastructure errors remain errors.

**Accept:** differential singleton include/skip/revert/OOG/zk-discard cases,
missing-read mutations and execution-root mutations have the pinned decisions.
A host Engine result alone never authorizes omission.


### Reuse inventory (2026-11-20)

This is integration work, not a new EVM or blob-verification implementation.
The existing stateless path already checks ancestors, header/body/receipts and
post-state root using the witness DB and native executor. The current Alethia
pin `59e06d0` has no Etna fork, so a later timestamp alone cannot select it.
An inspected upstream candidate `efc3c162` supplies timestamp-gated strict
committed-list/anchorless execution and a devnet override; it is not an adopted
runtime pin. Raiko2 still needs the matching trusted fork profile and narrow
scoping of legacy zero-root, first-successful-receipt and anchor assumptions.
The old `efc3c162` candidate's 2 s fee target was an audit finding, not the
latest source: #248 at `ad584325` now selects 4 s for Etna, including the first
Etna block. This resolves that source difference only. The PR remains unmerged,
and its runtime/API/SDK compatibility has not been tested in Raiko2. Network
timestamps are configuration, not a reason to duplicate the upstream executor
or trust a host fork override.

The provider's Beacon fetch/order path and both guests' shared KZG/versioned-
hash checks are reusable. Etna's remaining DA seam is strict version-6 payload
framing/decompression and exact ordered item equality with the matched range,
using the same verified hash list in the landing statement. Reuse the blob
unpacker, with Etna padding/full-chunk checks; do not route Etna through the
Unzen manifest's offset/default-on-error semantics. Pending #22422 makes the
first field header's high-bit check explicit; adoption and client/guest parity
remain P01/P05 tasks, not verified production support.

## P05 — Complete range relation and settlement output

**Files:** range/statement modules in `crates/protocol-etna`, existing guest
composition, and new `consensus_range.rs` / `land_statement.rs` integration tests.

- [x] Derive terminal h plus each interior signing-set endpoint from supplied sets and verify each required native quorum under its corresponding keys/powers and full deployment domain. Identical keys/powers across metadata changes add no target.
- [ ] Authenticate the actual origin/recovery schedule and adopted QC field policy before composing those native checks into the final range proof; supplied sets alone are not committee authority.
- [x] Compose the bounded complete Comet/EL header-hash chains and original envelope/EL bytes against the caller-authenticated origin/terminal expectations; preserve the generation-origin exception.
- [ ] Join that byte projection with P03's authenticated context and schedule, current/next-validator hashes and the required native QCs over the same headers. Do not add stock full-part-set/embedded-LastCommit checks to the revised hash-link relation. A later committee cannot replace the outgoing committee's endpoint certificate; origin p does not require a QC merely because it is an input boundary.
- [ ] Authenticate and decode ordered blobs and references against the complete supplied range; preserve all per-block anchor, ruleset, stop, FI and execution checks.
- [x] Provide the pure 21-word preimage/digest and ordered cut-hash encoders (ENC-A-29/30/35). The statement recomputes nested commitments from the same generation, p/h, blobs and supplied cut leaves.
- [x] **P05-C01 — Range/statement projection:** bind all shared full-width domain/origin/terminal fields, the matched terminal header state root and each interior cut to the existing range projection and supplied uint48 cursor sequence; return the existing statement digest. This checks cursor shape, not actual FI consumption or L1 authority.
- [ ] Derive and authenticate p through h cuts and terminal values from the actual verified range, with every interior height in order. Bind the computed bytes32 statement digest to the final proof using the pinned backend framing; add no ValState field.
- [ ] Preserve overlapping settlement and historical origins without reproving a suffix. Reject wrong leaves, blobs, references, generations and cross-fork boundaries.

**Accept:** singleton, no-handover and multi-handover ranges; range ends immediately
before/at/after a handover; missing outgoing/terminal QC; same-set recurrence;
interior heights without detached QCs; omitted/duplicate/swapped leaves; `p=c` and
`p<c<h`; `c>=h` cancellation; and reference expiry. Validity is established for
all `(p,h]`, not merely the newly settled suffix.

### Range/statement projection evidence (2026-11-20)

`range::verify_range_statement_projection` composes existing range projection,
`cut_leaf` and `LandStatement::try_digest`. The existing per-block verifier now
retains its already-checked Comet hash for that actual cut caller. It introduces
no hash pass, EVM/trie/KZG implementation, dependency pin, manifest/lock or
production guest/ELF change. Shape validation remains shared with the original
encoder, and errors preserve their source chain.

Compiled semantic RED preserved all 18 original range tests while the nine
new tests failed at the explicit composition seam. Final native evidence is
95 protocol tests (nine new), five independently authored signed-fixture
checks, strict all-target Clippy, workspace fmt and a locked standalone no_std
consumer. Its 221 normal runtime dependency rows, excluding host proc-macros,
enable no std/use_std/use-std. Independent source review and behavioral
verification passed. Two test-only Clippy fixes changed no production bytes.

Equal-count substituted cuts that the old shape-only encoder accepts now
reject, including changed height, EL/Comet identity, cursor and order. Full-width
domains, historical proof origins, singleton and uint48 boundaries, wrong
terminal state root and resealed broken ancestors have direct controls.
Supplied cursor validity, execution, DA, L1/committee authority, phase-one FI-off
profile and backend proof framing are still composing-verifier obligations.
The result is not a full certified range or proof. No guest cycles, SDK build
or real proof was run. R06/R07 and #22423 authority review remain open.

### P05-D01a — Decompressed execution-list equality (2026-11-22)

[`verify_decompressed_execution_payload`](../../../crates/protocol-etna/src/payload.rs)
checks the canonical outer RLP list prefix followed by each original projected
execution-item span in order, with no second full payload allocation or
transaction/header decoder. It rejects empty ranges, mismatched bytes and
lengths, and actual or expected encodings above 64 MiB including the prefix.
The caller still authenticates the complete range, bounds inflation during
execution and establishes strict compressed-stream and ordered blob provenance.
D01b, execution, anchors, FI-profile authorization and final guest/proof
composition remain open; this is not complete DA verification.

Acceptance evidence for this bounded component:

- Compiled semantic RED: all seven initial payload tests failed at the explicit
  unimplemented seam before implementation.
- `cargo test --locked --offline -p raiko2-protocol-etna --all-targets -- --test-threads=2`:
  101 maintained tests and three independent scratch tests passed, including
  seven historical earlier-relation controls. The six new maintained tests
  cover exact original empty/signed spans, count/order/substitution, canonical
  framing/truncation/extra roots and the exact inclusive 64 MiB boundary plus
  one-over actual and expected sizes.
- `cargo clippy --locked --offline -p raiko2-protocol-etna --all-targets -- -D warnings`
  and `cargo fmt --all -- --check` passed.
- A locked standalone `#![no_std]` consumer exporting the new API passed
  `cargo check --locked --offline --lib`; the normal runtime feature graph
  excluding host proc-macros contained 221 rows and no std/use_std/use-std.
- Independent adversarial source review and independent behavioral verification
  passed. Checks used immutable offline snapshots and the existing shared target
  with a 15 GiB disk reserve. No manifest, lock, backend, guest entrypoint,
  production ELF or proof-output change is included.

## P06 — Thin host and backend integration

**Files:** existing extension points in `crates/provider` / `crates/pipeline`,
adding Etna provider code and `crates/pipeline/src/forks/etna` when needed; new
`crates/guest-common/src/etna.rs` and existing `crates/prover` codecs/backend seams.
No Etna provider/pipeline fork module exists at checkpoint `845ecbcb`.

- [ ] Define and review the concrete complete guest-input/output types from P02–P05 before wiring collection. Use the existing Preflight/Validation/PipelineSpec and GuestInputCodec seams; do not invent a generic Protocol framework.
- [ ] Collect raw consensus/execution/L1/FI/DA material, distinguish missing history from authenticated contradictions and feed the same guest relation on both backends.
- [ ] Bind task/cache/artifact identities to range origin, generation, rules/spec ABI, image and ordered content/reference; preserve retry/restart and cancellation behavior without reinterpreting existing Unzen proposal IDs.
- [ ] Reuse the existing request/provider/pipeline/queue ownership and update API/config documentation only for actual surface changes.

**Accept:** collection-to-native-verifier replay, stale-generation/image/reference
and cached old-layout rejection, partial collection/retry, task restart and Unzen
request compatibility. Do not count collection as guest authentication.

## P07 — Guests, aggregation and actual proof acceptance

**Files:** existing RISC0/SP1 entrypoints, aggregation callbacks, prover backend,
xtask guest builder and tool-generated ELF/VK/provenance outputs.

- [ ] Freeze aligned host/guest locks and current crypto patches. Use normal build entrypoints once per backend after source review; record exact compiler/SDK/image identities.
- [ ] Wire deserialize → shared verification/execution → exact backend public-output encoding with the existing guest architecture. Authenticate the statement digest in that encoding, not the 672-byte preimage as a substitute. Validate fixed-range child proof identity and continuity; arbitrary child boundaries must not introduce extra mandatory QCs or a new landing grid.
- [ ] Execute representative valid and mutated invalid cases in both actual guests; generate and verify real proofs for both supported ZK backends and a permitted two-proof L1 verifier combination in an isolated test.
- [ ] Measure a worst-case valid single block, including maximum legal committee/registry/witness data and any full-budget FI trial, plus target multi-block/blob ranges. Record SDK/hardware/resource evidence and unavailable counterpart-prover requirements explicitly.
- [ ] Regenerate production ELF/VK/provenance through the documented tooling and verify source/artifact bindings. Native execution and mocks are not proof acceptance.

**Accept:** actual valid proofs and invalid-case rejection, matching statements,
child ordering/image/generation failures, single-block completeness and supported
batch limits. No IVC, new zkVM, new pricing or broad performance campaign is part of this milestone.

### Production ELF source provenance gate

CI's RISC0 source fingerprint mismatch is separate from native test results.
The fingerprint covers the production guest's local source closure plus the
whole root manifest, backend guest manifest/lock, toolchain inputs and all build-tool source,
including its tests. At `aeda3262`, against comparison base `3f4ec387`, four hashed
inputs changed:
`Cargo.toml`, `guests/risc0/Cargo.toml`, `guests/risc0/Cargo.lock` and
`xtask/build-guest/src/lib.rs`. Replacing just those bytes with base versions
reproduced the recorded fingerprint. Both checked-in RISC0 ELFs still match their
provenance digests and are unchanged; no current-source rebuilt bytes or image ID
are established. The Etna crate is not yet in that production guest's closure.

The later diagnostic cleanup restored both production guest manifests and locks
to comparison base `3f4ec387` byte for byte. Root manifest and build-tool source
still differ, so both production provenance records remain a merge gate.
P07 must run `just build-guest all --force` on the final source, review generated
ELF/VK/provenance changes, then pass `just build-guest all --check`. Do not hand-edit the recorded source fingerprint
or weaken the check. A legitimate rebuild may leave ELF bytes identical while
refreshing provenance. The stale source check runs before artifact verification
and RISC0 precedes SP1, so its first failure does not establish that SP1 artifacts
are current.

## P08 — Final review and ready gate

- [ ] Run required affected fmt/lint/unit/integration lanes and Unzen regressions, with current exact sources and proof/ABI conformance.
- [ ] Obtain independent whole-stack adversarial review and behavioral verification; fix and have the raiser recheck all material in-scope findings.
- [ ] Record remaining assumptions and external gates; only mark #305 ready after actual callers, production artifacts and proof acceptance satisfy P01–P07.
- [ ] Keep deployment, image publication/registration, governance actions and merges outside this implementation plan unless separately authorized.

## Updating progress and preserving history

For each completed item, update its checkbox and milestone row together, recording
commit/PR, exact source pins, executed checks and review evidence in this plan or
a linked tracked artifact. A changed source/ABI can reopen a milestone; keep its
old evidence labeled historical. Do not mark a whole milestone done because one
fixture or host-only test passed. Update #305's current-status link/summary when
these states change; this table remains the single source of progress status.

Legacy mapping: old Task 1 is the rejected MPT prototype; Task 2 / Task 3a and
patched-SDK preflight map to P00; the `845ecbcb` slice maps to partial P02; old
Task 3b context maps to P03, Task 4 to P04, Task 5 to P05, Task 6 to P06/P07 and
Task 7 to P08. The previous plan remains available in Git at `845ecbcb`; the
design document and diagnostic README retain its source/checkpoint provenance.
