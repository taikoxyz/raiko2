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
