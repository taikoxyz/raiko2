# Honest minimal storage-access growth under frozen node pricing

This is the second authorized pressure-test axis: state/input growth, retaining identical-proposal
actual SP1 PGU versus node-per-block/literal-sum and full proposal estimates, independent deficit
flags and all own-N0 controls. It is independent of the45-case warm-journal/hash composition panel.
No candidate price is fitted or proposal cost allocated back to mining.

## Existing construction and finite matrix

Use ordinary ProgramRequest with existing ProgramStorageState, one block and one default EIP-1559 user CALL,
empty calldata,15M user EVM gas and evm-gas-only profile. Omit the explicit transactions descriptor:
existing storage_state is one-block-only and rejects that descriptor. Keep native b049, quotec10,
modelc893, sampler1998/SDK6.3/ELFb76 and current artifact bindings. No guest/Rust/toolchain build.

Prestate has8192 or16384 logical storage keys0..size-1, all with value1. The existing retainer
constructs only the target multiproof; source-state size is real background state, not witness
padding. All slot preimages are supplied to the canonical builder, never arbitrary hashed keys.
Warmup reads slot0 and requires value1. Bulk access modes:

- warm: key0 every round;
- present: keys1..N, all value1;
- absent: keys1048577..1048576+N, all outside the initialized key range, all value0.

One generic fixed64-site body computes `base + stride*i` (warm0/0, present1/1, absent1048577/1),
executes SLOAD, adds the returned value to a memory checksum and increments i. Same body/PUSH
widths across counts/modes; only fixed-width immediates differ. Counter and checksum occupy
separate words. At termination require i=N and checksum=N for warm/present or0 for absent,
then STOP. The once-only warmup does not contribute to the checksum.

proof_slots is exactly0 plus bulk distinct keys, deduplicated; warm has only slot0. No insert or
delete targets, storage writes or added account/code witnesses. Prestate remains unchanged.
Native/quote verification must confirm warmup is the only initial cold access for warm mode,
and each present/absent bulk key is new/cold. Node observed unique-resource increments are
checked independently of opcode totals; model prices cannot stand in for actual path observations.

Selection:3 modes ×2 prestate sizes ×N0/64/1024/4096 =24 distinct inputs, plus2 warm-mode
N65536 inputs, one per prestate size. Total26. Cold N is bounded4096, warm N65536; all counts
multiples64. Fixed prestate and own-N0 controls remain within each mode/size series. State-size
comparisons retain different authenticated roots rather than assuming equal inputs.

## Interpretation

This panel tests cold-path traversal and minimal-witness/input growth versus warm-cache controls.
The witness trie is reconstructed once; subsequent accesses do not imply repeated materialization.
It is not the separate cold-child-REVERT warm-set path and does not change honest-input scope.
A successful receipt/sum/count alone can hide wrong keys; independent concrete key/value/order,
proof target/prestate, trie-root and resource oracles are required. Test wrong sequential base,
wrong stride and missing proof target, including a mutation that can pass final checksum.

Compare literal actual/node and actual/full including N0, matched within-mode/size growth and
state-size effects. Do not compare raw Unzen scores with PGU, erase failed guards, attribute all
remaining difference to startup or claim a universal depth/workload bound. Any scalable gap
gets explicit source attribution and a separate successor/untouched validation cycle.
