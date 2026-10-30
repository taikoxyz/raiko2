# High-count minimal DELEGATECALL

## Question and scope

The frozen a298 model charges about 3,841.907 PGU for a confirmed DELEGATECALL,
much less than the conservative CALL and STATICCALL wrappers. Earlier finite
experiments emphasized memory expansion, returned bytes and nested rollback.
This checkpoint isolates high repetition of tiny child frames without large
memory, returned data or per-iteration creation costs.

The earlier retained CALL-memory and copying cohorts reached at most 512 loop
iterations, with additional dispatch/header or memory work. They do not replace
the direct 32,768-call density check. STOP and empty RETURN are the inexpensive
candidates. Empty REVERT and INVALID are intentional failure-path controls:
their existing terminal/failure pricing has a 5M PGU event floor, so the cheap
DELEGATECALL wrapper alone does not describe those complete quotes.

Keep literal actual proposal SP1 PGU / node-known zkGas as the ranking metric.
Complete proposal quotes and matched same-runtime repetition increments are
separate diagnostics. Startup-dominated controls do not establish scalable
underpricing. Measurement decides whether pricing needs to change.

## Finite construction

- One block, one user CALL, empty calldata, 15M transaction gas and the explicit
  evm-gas-only execution profile; ordinary validity checks remain active.
- A parent creates one tiny child before the loop, including in the N=0 control.
  Child runtime is STOP, RETURN(0,0), REVERT(0,0), or INVALID. The created address
  must be nonzero. No extra child account or code is added to prestate; a code
  byte string already used by a system account need not have a unique hash.
- Repeat DELEGATECALL N=0,1,1024,32768 with exactly 64 forwarded gas, zero input
  and output lengths, and fixed-width counter immediates. Check each return
  status against its expected success/failure and require empty return data.
- The parent always reaches STOP. Reverted/exceptional children are contained;
  exact completed opcode maps, user receipt, account nonce/runtime and spawn
  counts gate native qualification. INVALID itself is not a completed opcode.
- All 16 plans, controls, rejections and measurement failures remain visible.
  Do not remove a case merely because an old Unzen cap would have rejected it.

Frozen constructor b049, quote 0ead, sampler 1998, ELF b76, model a298 and their
existing source bindings are reused. No Rust/guest rebuild or tariff refit is
part of this measurement checkpoint. Terminal variants are distinct programs;
only within-terminal repetition changes provide matched scaling comparisons.

## Related source triage

Pinned WitnessDB uses indexed ancestor hashes and REVM uses a 256-slot block
hash cache. Warm account/slot accesses hit cached state; code is loaded once.
These facts reject a proposed repeated linear-history/MPT-materialization
mechanism, but do not validate their numerical tariffs. BLOCKHASH and warm-SLOAD
numerical checks remain follow-ups after this focused frame experiment.
