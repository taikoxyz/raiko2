# Multi-account code loading experiment design

The user authorized the next block-aware workload campaign: grow account count,
unique code hashes and code length while each callee executes a short normal
RETURN path. Preserve literal actual/node ratios and the three distinct totals:
node per-block estimate/literal sum, full proposal estimate and actual identical-
proposal SP1 PGU. Within30M charged EVM gas, node shortfall strictly above2B PGU
is high priority; smaller discrepancies can remain TODOs. Keep honest minimal
witnesses, controls, semantic negatives and bounded failed attempts.

## Scope and approach

Extend only the feature-gated host fixture and its case generator/tests. Current
ProgramRequest supplies one runtime account; add optional prestate contracts for
this campaign. They represent previously deployed code, not deployment inside
the measured block. Each extra contract starts with nonce1, zero balance and
empty storage. Preserve the ordinary primary runtime, sender and system setup.
Creation/setup costs are outside this measured prestate-call panel and must not
be claimed free for an attacker over the entire lifecycle.

The host emits the existing GuestInput format. No guest, SDK, ELF, shared input
type, runtime price or fork/admission limit changes. Rebuild/freeze the host
fixture once; reuse quote c10, modelc893, sampler1998 and SDK6.3 ELFb76 unless an
actual incompatibility is found. Do not patch vendor code or add adapter layers
to force a quote/input to pass. Retain published fixtures and frozen artifacts.

Alternatives: CREATE setup in the same block measures deployment and can obscure
the reusable predeployed-code path; arbitrary per-user calldata is unnecessary
because the parent can derive target addresses. A new guest format is unnecessary.

## Host fixture interface and eligibility

Add `program.prestate_contracts`, an optional list of `{address, code_hex}`.
Addresses must be unique and avoid the primary/sender/system accounts and the
reserved low address range. Code must be bounded, nonempty and valid for the
current interpreter. Limit to512 extra contracts and24576 bytes per contract.
Reject duplicate/conflicting addresses, malformed values and oversized totals
before publishing outputs. Avoid a second execution/quote codepath.

This initial mode supports one block and explicit CALL users only. It is mutually
exclusive with storage_state, access_list and authorizations, and with CREATE
envelopes. All signed users still share the ordinary input/to/gas declaration.
Keep old missing-field behavior, request limits and output bytes unchanged.
Allow a32MiB request only for the new mode; legacy requests retain the4MiB bound.
The existing128MiB output guard remains. These are fixture limits, not protocol
or proving-capacity claims.

The full prestate is needed to construct its authenticated root; it is not the
final witness. Obtain actual loaded account/code/storage targets from native
execution, derive a canonical minimal multiproof, deduplicate code by hash and
replay that input to verify the same result. Reuse existing trie/proof and state
collection facilities. A preparatory native pass is allowed and is not an SP1
actual. Do not select targets solely from an unverified operator declaration.

Unused background contracts/code must not enter the final witness. Code shared
by many addresses appears once. Account proofs and warmth remain address-based;
each CALL executes its own frame. Capture configured versus witnessed account/
code counts/bytes and full versus minimized state-node counts in a new optional
host report section. These diagnostics are not per-opcode or per-tx actual PGU.
Only the new mode gets changed witness construction; old requests remain exact.

## Finite case matrix

Use4 successful signed user CALLs per block, with no external RPCs. A parent
derives addresses and cycles over target accounts. Callees execute the same
ADDRESS/store/RETURN32 prefix; parent verifies each returned address, exact return
length/status and its ending counter. Unreachable bounded tails vary code length
and, for unique-hash cases, a fixed-width salt. Callee execution maps are equal
between shared-code and unique-code cases. Do not use the conservative REVERT
reserve to cover missing code work.

Engineering matrix: target counts16/128/512 × code lengths32/4096/24576 ×
shared/unique code × own-N0/full =36 inputs. The one-target configuration is a
native-only construction/cache control, not a duplicate shared/unique actual.
The exact unroll and integer full bound are implementation choices verified by
gas arithmetic. Full targets must use at least99% and at most30M charged gas,
including Anchor, with the existing31M header. Keep the same full-selected user
envelope in each zero/full pair and verify every remaining header-gas prefix.

N0 performs no target calls: its unused prestate code must be absent from the
witness. Full must visit all configured targets; repeated visits test warm and
code-cache behavior. New user transactions reset EVM warmth while block-level
code objects may remain cached. Gas predictions separate intrinsic, call cold/
warm cost, memory/copy, opcode/loop overhead and any floor/refund effects.

Keep code bytes, executed bytes and unique hashes distinct. A large unreachable
tail contributes loading/analysis work without being executed. Unique-code salt
selection must change hash without altering the executed prefix, instruction
histogram or calldata. Honest minimal inputs exclude duplicated unused code or
unrelated proof padding. Same-length shared/unique comparisons are diagnostics,
not isolated causal coefficients or universal bounds.

## Verification and resource constraints

Host validation, legacy byte-compatibility, missing proof/code failures and
new witness minimality need focused regression coverage and independent review.
Verify cache/analysis distinctions from actual witness bytes/identities, not an
assumed count from configuration. Retain semantic mutations that can pass EVM
receipts/maps but violate target/order/code expectations, as well as native
completion rejects, with no fabricated PGU.

Use the existing shared Cargo target, bounded jobs and cached host test profile;
do not delete shared caches or build guests. Check disk before compiling. Freeze
committed source, new native SHA and all36 qualified canonical inputs before any
SP1 sampling. Validate small multi-account inputs with the retained ELF first.
Run actuals sequentially using existing6GiB PSS/600s/output/log/free-space guards.
A failure/censor remains unknown. No implicit retry or renamed failed sample.

Independent testing must qualify all36 native/quote inputs, exact signed users,
body/operand/target semantics, gas and minimal authenticated state/code coverage.
Independent actual auditing checks SDK normalization, canonical hashes/public
outputs, literal and full quotes,18 own-N0 increments and owned-resource cleanup.

Keep tools/tests/usage and research records in pending Raiko #296. Retain only
data and provenance in a new immutable Atlas fixture revision/PR after auditing.
Reth #260 and Raiko #284/#296 remain pending for the future fork; no automatic
merge, deployment, activation, repricing or limit adjustment. Multi-account
storage mutation/deletion/ancestor rollback follows only after this code panel.

## Additional comparison after observed CALL reserve masking

The first36 CALL observations are frozen and retained. Their full quotes are
dominated by the existing confirmed-CALL wrapper:1M PGU plus64 dispatch per event.
For example108780 calls contribute108786961920 PGU, while the proposal's actual
SP1 workload is1300643989. This conservative quote cannot isolate code-loading
underpricing. It does not justify changing prices during validation.

Add a separately frozen36-input DELEGATECALL matrix through an optional semantic
generator argument. Keep the original CALL requests, IDs and metadata compatible.
DELEGATECALL's existing wrapper is6000 PGU plus64 dispatch; avoid the unrelated
CALL reserve while retaining every opcode/frame and its actual work. The matrix
axes, four top-level CALL users, gas/domain/resource rules and18 own-N0 pairs remain.

Child ADDRESS returns the parent context for DELEGATECALL; assert that context
in the parent and independently authenticate actual code-target addresses/order,
nonce/balance/storage/codehash leaves and supplied code bytes. Do not claim that
returned ADDRESS proves the code account. Recalculate exact route maps and gas,
with265 rather than272 gas per additional warm call. Keep old and new source/
input/configuration freezes explicit in the final two-panel data fixture.

Original CALL actuals finished before integrating the optional generator change.
No guest, SDK, ELF, host rebuild, pricing or admission-limit changes are required.
