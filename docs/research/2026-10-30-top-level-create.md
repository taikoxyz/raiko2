# Top-level creation workload design

## Objective and scope

Continue the approved search for `actual SP1 PGU / new node-known zkGas` after
access-list preexecution. Complete quotes and matched increments are separate
diagnostics. Measure valid post-Unzen transactions with honest/minimal witnesses;
retain controls and native exclusions. This round changes host fixture tools only:
no guest rebuild, tariff change, production admission or deployment. EVM-gas-only
single-block saturation remains a later experiment.

Top-level creation has no preceding CREATE opcode event. Transaction and payload
fees may already cover its setup/initcode analysis; measurement must establish
whether there is an omitted cost before changing any price. Ordinary CREATE derives
its address from sender/nonce, not a hash of initcode (CREATE2 is different).

## Chosen extension

Extend the existing optional dimensions of ProgramRequest rather than building a
second constructor or patching serialized GuestInput. Add optional
`program.transactions` with `kind` (`call` or `create`), `count_per_block` (1..2048),
`gas_limit` (21000..15000000) and optional `expected_runtime_hex`.

Omission preserves the existing single call,15M gas, canonical input and report.
Calls retain1..24576-byte runtime and at most1MiB calldata. For creation, `code_hex`
is0..49152-byte initcode and `input_hex` must be empty. Initcode belongs to signed
transaction input, never prestate/witness code. Create addresses are absent initially.
If `expected_runtime_hex` is supplied, each created account must be present with
nonce1 and the expected code hash after execution; omission expects the address
absent (for example failed creation). A runtime expectation is invalid for calls.
Bound expected runtime to24576 bytes. Receipt status is still independently checked
against `expected_user_success` for every user transaction.

The explicit descriptor initially excludes storage-state, authorization and
access-list dimensions; their existing omitted-descriptor path stays unchanged.
This avoids silently applying one-user-per-block reports to repeated transactions.
Future composition can extend the same typed path with its own gates.

All repeated transactions use one signer with monotonically increasing nonce over
all blocks. The explicit descriptor uses the protocol1Gwei maximum-base-fee cap as
max_fee_per_gas and a fee-neutral initial/final sender balance of10^18 wei; this
covers15M*1Gwei upfront and avoids excluding dense later blocks at the old25M fee
cap. The omitted path retains its old25M cap and10^16 balance byte-exactly. Keep31M block gas,30M manifest gas and the existing15M maximum per-tx
budget. Explicit smaller gas limits allow repeated cheap transactions without the
15M declared-limit packing artifact. All expected candidates must be included in
exact signed order; filtering, gas exhaustion, wrong nonce, wrong receipts and
one-blob overflow reject before artifact publication. One-blob overflow is a
construction transport bound, not a model/admission failure. These harness bounds
do not establish coverage of every legal block envelope or saturated capacity.

Observe every user transaction and exclude Anchor, retaining exceptional and
reverted work. Native reports aggregate all user receipt gas per block/proposal and
report raw signed transaction identities, nonces, gas limits and create addresses/
poststates in a new optional `transactions` section. Carry actual committed code
into later block witnesses; never preload speculative code or accounts. Reuse the
existing canonical serialization, production reconstruction and full proposal quote.

## Measurements

Freeze a finite producer with ordinary-call count controls; empty/STOP/RETURN/
REVERT/exceptional creation; initcode lengths0/1/32/4096/16384/49152; unreachable
zero/JUMPDEST/PUSH32 tails; within-block repetition; and multiblock/state-continuity
controls. Keep successful code deployment distinct from empty-runtime creation.
Use equal-count/equal-block references for length/pattern pairs. Include matched
call/creation count slopes as diagnostics without claiming their different state
work is identical. Aggregate proposal actual PGU, never fabricate per-tx actuals.

Native-qualify all cases before SP1. Reuse the frozen b76baf2e/SDK6.3 program,
its original4580 source manifest, current3794 model and the guarded6GiB-PSS sequential
sampler. Freeze new host binaries and their complete source inventories, clearly
separate from the old guest source. Independently verify canonical signed inputs,
counts, receipts, trie states, raw results and source bindings. Reusable tools and
usage stay in Raiko; immutable observations go to Atlas after independent audit.


## First completed observations

The44 selected cases all completed actual execution on the unchanged b76baf2e /
SDK6.3 program, producing44 distinct canonical inputs,56 block occurrences and
2814 transaction occurrences (2758 users). Complete source review and independent
native qualification passed before measurement; the native creator is
`f216a2843234f812b92c8c8e721143db20192ed1`. Its484-file compiled-source closure
retains the original modified458ebc1b/null build identity and a separate later
commit association. The unchanged quote retains its own481-file/source118eeedb
identity; the guest retains its original67-file/source4580 identity.

All44 raw observations passed independent evidence-integrity review; all176 owned
SP1 resource objects were absent afterward. Peak process-tree PSS was1,844,956,160
bytes. There are no measurement failures. Pricing accuracy is a separate result:
one complete quote underestimates its actual, as shown below.

| Controlled case | Actual PGU | Node-known PGU | Complete quote PGU |
| --- | ---: | ---: | ---: |
| One ordinary STOP call | 160,983,458 | 6,109,740 | 161,051,571 |
| 512 ordinary STOP calls | 247,377,051 | 108,649,741 | 263,591,572 |
| One empty-initcode creation | 160,968,292 | 6,104,620 | 161,034,451 |
| 512 empty-initcode creations | 259,798,925 | 112,243,339 | 374,956,234 |
| Return/deploy0 runtime bytes | 160,975,808 | 6,129,673 | 161,059,504 |
| Return/deploy24576 runtime bytes | 161,530,410 | 6,243,393 | 161,173,224 |
| Return24577 bytes; included failed deployment | 160,976,696 | 6,191,350 | 161,106,669 |

The successful24576-byte deployment has a357,186-PGU complete deficit. Relative to
its same-envelope zero-runtime control, actual growth is554,602 PGU while exact
fixed-point quotation grows113,719.544968474130133552 PGU. Runtime coverage is not
missing: both node and exact resources include all24576 additional unique code
bytes and one hash, priced52,123 PGU; RETURN/operation growth contributes another
61,596.544968474130133552 PGU. Distinguish an inadequately priced processing path
from an absent resource counter.

Same-length code-content variation remains visible too. At49152-byte initcode,
JUMPDEST tails add205,860 actual PGU versus zero tails with identical quotes. The
4096-byte counterpart adds16,438 PGU at unchanged price. Their complete quotes
remain covered, and overall length growth is covered by existing byte pricing.
Do not claim every matched marginal difference is covered or infer that each such
variation requires its own selector.

## Source-supported followup

The frozen program's manifest-bound lock selects REVM43.0.3. On successful create
completion, `return_create` first validates result, code length/prefix and deposit
gas, then creates a fresh legacy Bytecode, analyzes it, hashes its original bytes
and stores it. Size-limit failure at24577 returns before that preparation. Each
new Bytecode has a fresh hash cache; code-hash resource sets deduplicate equal
outputs after this work. Source inspection supports a per-occurrence hypothesis,
not an attribution of every observed PGU to a single hash operation.

RETURN pricing already handles dispatch, returned length and memory expansion;
it cannot distinguish CALL return from accepted deployment. Top-level creation
has no preceding CREATE opcode, while internal CREATE/CREATE2 have confirmed-spawn
fixed charges. Do not broadly increase RETURN or unique-code-byte prices from one
boundary sample.

A separate38-row development followup tests intermediate sizes, repeated identical
runtime, matched ordinary CALL-return controls and cheaply generated runtime bytes
(0x00/0x5b/0x7f via equal-opcode MCOPY expansion). It includes35 new inputs and three
explicit known reference replays; it is not a post-fit holdout. New measurements
must precede selecting a correction. Keep one prospective validation set unopened
until a candidate policy is fixed. If needed, a processing selector must count
actual accepted create-frame processing per occurrence, retain work later reverted
by an ancestor, and cover both plain/inspected paths exactly once. Final committed
state or unique hashes cannot establish that event count by themselves.


Multiblock runtime comparisons additionally include the constructor's carried code
in later witnesses. Those codes preserve execution continuity but need not all be
read in a fresh-address creation workload; witness initialization eagerly prepares
provided code. Keep these as explicit constructed-witness/state-continuity
diagnostics, rather than attributing the entire difference to deployment or using
them as strictly minimal-input calibration. Within-block N1/N2/N4 comparisons do
not have that carry-forward confound. Check ordinary loaded code separately before
choosing a narrow deployment-only correction.
