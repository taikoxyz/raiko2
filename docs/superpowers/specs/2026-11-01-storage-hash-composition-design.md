# Storage/hash composition under frozen node pricing

The authorized next stage tests composition, state/input growth and untouched validation while
retaining three quantities for each identical proposal: node block estimates plus their literal
sum, the complete proposal estimate, and actual SP1 execute-only PGU. Both underpricing flags,
absolute shortfalls, controls and failures remain. Proposal allocation and capacity policy stay
undecided. This first finite composition panel targets journal work after repeated cheap writes.

## Construction

Reuse the existing Python assembler, ordinary ProgramRequest, frozen native constructor b049,
quote c10/model c893, SDK6.3 sampler1998 and source-bound ELFb76. No Rust/guest build, dependency
or tariff change. One user program account initially has empty storage. Parent checks slot2=0,
seeds5, and eventually clears it back to0. No custom prestate, synthetic proof padding or
precreated child is supplied. Thus existing multi-transaction/block construction works unchanged.

The shared kernel checks entry slot2=5, writes7, then executes a fixed64-site unrolled body:

- `dirty`: set slot2=11 then7 per round;
- `hash`: KECCAK256 of32 zero memory bytes and discard per round;
- `mixed`: set11, hash the same32 bytes, then set7 per round.

The original transaction value is0, while all bulk writes alternate two nonoriginal nonzero
values. The setup cost is constant; bulk writes exercise the cheap dirty-rewrite branch rather
than repeatedly restoring the original value. End guards require slot2=7, restore5, and place
the round count at memory128. Counter memory64 stays outside the hash input memory0..31.
A once-only known-zero-word KECCAK result guard is present even at N0. Independent qualification
must verify actual slot/value operands, hash bytes/length, counter and semantic results.

Layouts: `direct` executes the kernel in the parent, then cleanup/STOP; `return` self-DELEGATECALLs
once and returns32-byte marker data; `revert` makes the same self-call and catches a REVERT with
the same32-byte marker. The outer transaction always succeeds and checks child status, return
length/marker, restored slot2=5 and final cleared slot2=0. Parent input flag is at memory256;
child receives32 calldata bytes. Forward12M gas, with the same15M per-user EVM limit and31M
block envelope. Native actual gas and exact terminal/count gates exclude OOG/short execution.
Parent memory is preallocated before calls; frame/memory differences remain in own-N0 baselines.

Parent entry SLOAD/seeding prewarms slot2, and the self-call target is already warm. This
panel therefore tests warm dirty storage journal work; it does not cover first cold child
access followed by REVERT removing the warm-set entry. Keep that distinct path as a followup
with its own price/control interpretation.

No operation per-round guard is introduced beyond the loop and body. Code and PUSH widths stay
fixed within each kernel/layout/count series, including the unused N0 body. Counts are proposal
aggregates; inherited runtime/guard metadata explicitly identifies its per-user scope.

## Frozen finite selection

-27 base inputs:3 kernels ×3 layouts × rounds0/2048/32768; one user, one block.
-12 additional packing inputs: mixed kernel,3 layouts;4 users at rounds0/8192 or16 users at
 rounds0/2048, one block. High workloads retain32768 total rounds per block.
-6 additional block-growth inputs: mixed kernel,3 layouts, rounds0/32768, one user in each of4
 blocks. Total45 distinct planned inputs. All are new validation inputs under the already-frozen
 candidate; none chooses its coefficients. Guard/native/resource rejections stay visible.

## Interpretation and boundaries

Compare each workload to its own N0 at matching layout/user/block counts. Compare dirty/hash/mixed
increments and alternative layouts for ownership/composition diagnostics; retain literal whole
proposal actual/node and actual/full comparisons. No arbitrary subtraction of measured startup,
per-block actual attribution, universal bound or assumption that remaining deficits are startup.
Code lengths differ across kernels/layouts, so cross-program totals are not isolated opcode costs.

Success requires qualified semantics and complete evidence, not a particular result. A newly
scalable deficit gets source attribution and an explicitly separate candidate/holdout cycle.
Normal/high-price failure controls remain; no fitted adjustment is made silently on this panel.
The already-deferred unrestricted gas-capacity experiment stays deferred. Reth260/Raiko284/296
stay pending/do-not-merge. Reusable producer/tests/usage belong in Raiko; fixed data belong in a
new immutable Atlas revision. No old fixture or Release asset is rewritten.
