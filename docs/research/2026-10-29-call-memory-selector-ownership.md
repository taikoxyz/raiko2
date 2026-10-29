# CALL output memory selector ownership

## Finding and evidence boundary

The frozen `nested-return-{explicit,double}-l65536-d4-n64-b1` pair used identical
runtime bytecode and completed opcode counts. Enabling automatic call-output copying
changed user gas from 5,486,575 to 5,486,587 and actual SP1 PGU from 176,831,805 to 181,400,355,
but node-known PGU fell from 91,147,312 to 83,183,736. This is a confirmed metering
omission. Those measurements alone do not establish scalable workload underpricing:
complete proposal quotes remained conservative, and the previous repeated-count slope
within each mode was conservative. Retain both literal actual/node and complete ratios.

The examined model is `67d0b01fee7f5ba208dfd2311e5be469ec96421156ecf5f0c14962c27e7b4df9`,
Reth revision `5ac54f8c13e2e6c4c0755cfb3216503f21fd2e40`. Actual execution used explicit
ELF `416f5a475b17aa15914fb9387159d9eae9ecd474e7104da65ac22512ab56da2e`
and local SP1 SDK 6.3.0; production SDK parity has not been established.

## Exact accounting

`PendingStep::finish` in Reth's `crates/evm/src/zk_gas/runtime.rs` returns early
for selected CALL-family spawns, before `finish_model_input` calculates memory
selectors. The confirmed-spawn route is emitted without model input; generated
DELEGATECALL pricing has a fixed wrapper price and zero memory coefficients.

Automatic output copying expands caller memory during DELEGATECALL. Without it,
the following RETURNDATACOPY expands the same memory and charges the memory selector.
Moving the expansion between these instructions therefore changes its pricing owner.

The header starts with 6 memory words. Output offset 256 plus 65,536 bytes ends at 2,056
words. With `C(w) = 3*w + floor(w*w/512)`, expansion is 14,406 EVM gas. The root
grows once; three fresh nonleaf frames grow in each of 64 iterations: 193 growth events.
Using the model's integer-scaled shared-memory coefficients:

```text
growth_event       = 161142459292709706509
evm_gas_delta      =   2719407004771945406
boundary_4k_event  = 1925131900689577103907

193 * (growth_event + 14406 * evm_gas_delta + boundary_4k_event)
 = 7963575972450297939545636

explicit operations = 89328821671547197471006919
double operations   = 81365245699096899531461283
difference          =  7963575972450297939545636
```

This exactly accounts for the observed operations-price reduction. Payload, fixed
costs and other resource selectors were equal. It identifies where the cost disappears;
it does not independently calibrate the price of CALL output memory expansion.

## Retained follow-up

`bin/zkgas/bench/cases/SpawnMemory.sol` isolates fresh middle-frame CALL output
expansion with an empty leaf result and no RETURNDATACOPY. The finite generator
compares all four CALL variants, lengths 0 through 1MiB and repeated fresh frames,
with matched zero-length controls. Runtime status checks plus native exact STOP,
RETURN and CALL counts reject early OOG or accidental short execution.

`SpawnInput.sol` and `spawn_input.py` provide the complementary44 input-memory cases:
the inner CALL passes the fixed128-byte header plus a zero-filled range, with zero
output length. The leaf reads only its header and STOPs. This tests allocation and
input copying at the same selected-spawn boundary. Each family has separate matched
controls; bytecode remains fixed within each family.

Run through the retained toolkit described in `bin/zkgas/bench/README.md`. Preserve
native rejects and failed SP1 measurements. A pricing correction requires measured
scaling and a separate reviewed upstream change; this toolkit does not change tariffs.

## Implemented correction and checkpoint

[Reth #260](https://github.com/taikoxyz/alethia-reth/pull/260), commit
`0f30f488c88635e38932843e67af2d918319f7e7`, carries parent memory selectors on confirmed
CALL-family wrappers and applies the existing shared-memory coefficient values. No coefficients
were refitted. Raw/native gas remains absent from these wrapper events because it can include
forwarded child gas. CREATE/CREATE2 and legacy Unzen filtering are unchanged. The model identity
is `537a965e51dd0416e3472a31cca8b067a85723fe4bbd08c52027ef3ca73f0e3b`.

Replaying the original two canonical inputs confirms that explicit mode remains at 91,147,312
node-known / 246,089,143 full PGU. Double-copy mode rises from 83,183,736 / 238,125,567 to the
same 91,147,312 / 246,089,143. This restores memory ownership and removes the price decrease.
Automatic return copying still does extra actual work; equal resulting prices do not establish
a separately calibrated automatic-copy term or exhaustive composition coverage.

The retained follow-up contains 88 plans, 80 actual observations and 8 distinct native exclusions.
Full deficits change from 4/80 to 0/80. All 36 measured zero-length controls are unchanged;
all 44 matched memory-growth increments are conservatively covered. The largest old
actual/full ratio was 1.012324. Source revision, original reports, controls, failures and both
price versions are preserved in [Atlas #9](https://github.com/taikoxyz/raiko2-atlas/pull/9).

Same-input replay of 50 retained Mainnet proposals / 9,600 blocks adds 690 PGU to one full quote.
Workload-weighted full/actual changes from 2.0851253703 to 2.0851253801, with zero full deficits
in both versions. External observations retain their SP1 v6.1.0 identity; controlled samples
retain local SDK 6.3.0. No new proving-throughput or production-program parity claim follows.

Next tests should separate actual returned-byte copying from reservation, including success,
caught REVERT and preallocated buffers, then combine frame depth, storage and rollback/exception
contexts. Continue retaining the literal actual/node controls and synthetic overquotes. The
native producer's multi-user-transaction capability remains a named extension, not implemented
coverage.
