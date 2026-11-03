# ZisK CPU execution experiment

This lane executes the complete saved Shasta `GuestInput` with the same
`prove_shasta_proposal` implementation as native/SP1/RISC0. It does not register
a backend, generate a proof, download proving keys, or change production zkGas.

The standalone workspace is `guests/zisk`, pinned to ZisK
`1e22630bb7e143c30705a1603ed87cfd3da8c6ec` (v1.3.2-alpha). Use the official
ZisK compiler supporting the current Rust 1.95 minimum; the experiment selects
compiler source `c83bf43be864c5c444d446c38f47d250d97761ca` (Rust 1.99).
Build with the corresponding `cargo-zisk build` command inside an isolated
container, selecting the `zisk-shasta-proposal` binary. The ZisK CLI provides
required target/linker flags; plain `cargo --target` is insufficient by itself.
No SP1/RISC0-specific patch overrides are added to this workspace. Existing
shared source pins, including `kzg-rs`, are inherited without claiming ZisK
acceleration. This first lane uses Rust crypto and a freeing dlmalloc allocator.

Keep the standalone `Cargo.lock`. Its Alloy macro/parser entries
match main's 1.6.1 types; unconstrained fresh resolution selected incompatible
1.7.3 macros during the first build. No shared dependency pins were downgraded.

The shared Taiko protocol intentionally selects C zlib. The build environment
must include the RISC-V bare-metal C compiler and its standard headers (for
example picolibc), and select it for this target:

```bash
CC_riscv64ima_zisk_zkvm_elf="riscv64-unknown-elf-gcc -specs=picolibc.specs" \
  LIBZ_SYS_STATIC=1 cargo-zisk build --release --bin zisk-shasta-proposal
```

Preserve that C backend; changing compression backends to bypass cross-compilation
would change the protocol implementation used by this experiment.
Both guests include a small C allocation bridge to the selected Rust allocator.
It preserves 16-byte alignment and the original allocation layout for `free`,
zeroes `calloc` payloads, and returns null on size overflow. Native tests keep C
symbol exports disabled so they cannot replace the host's libc allocator.

For strict offline/locked rebuilds, use the CLI-equivalent flags below from
`guests/zisk` in the prepared container. `ZISK_SOURCE_DIR` is the pinned SDK
checkout, not a locally modified linker script. The stable script path avoids
invalidating every Cargo dependency when the SDK CLI creates a new temporary
linker filename on each invocation:

```bash
zisk_build_dir="$(mktemp -d)"
cp "${ZISK_SOURCE_DIR:?}/ziskbuild/zisk_linker_script.ld" \
  "${zisk_build_dir}/zisk_linker_script.ld"
CC_riscv64ima_zisk_zkvm_elf="riscv64-unknown-elf-gcc -specs=picolibc.specs" \
  LIBZ_SYS_STATIC=1 \
  RUSTFLAGS="-C target-feature=+zba --cfg zisk_guest -C link-arg=-T${zisk_build_dir}/zisk_linker_script.ld" \
  cargo +zisk build --locked --offline --release \
    --target riscv64ima-zisk-zkvm-elf --target-dir "${zisk_build_dir}/target" \
    --bin zisk-shasta-proposal
```

Run the host benchmark in the same isolated, network-disabled container with
read-only source/tool mounts and a writable disposable output/cache directory:

```bash
cargo run --release -p zisk-bench -- \
  --input /fixtures/proposal.json \
  --elf /artifacts/zisk-shasta-proposal \
  --emulator /tools/ziskemu \
  --output-dir /results/proposal-run \
  --max-steps 1000000000000
```

The output directory must not already exist. The input is a complete JSON
`GuestInput`, including its carry data; this tool does not rewrite verifier
selection or fill missing carry. Native validation runs first. The tool then
serializes bincode 1.3.3 fixed-int data inside a single raw ZisK frame (8-byte
little-endian payload length, zero padding to an 8-byte boundary).

The emulator command is `ziskemu --elf ... --inputs ... --output ... --max-steps
... -X --no-thousands-sep --csv-separator , --save-stats ...`. Raw stdout, stderr,
input frame, 256-byte public output and CSV cost breakdown are preserved. Only
a successful process, exactly 64 public u32 slots, zero unused slots, exact
32-byte native/guest digest equality, nonzero steps strictly below the limit,
and a complete internally consistent cost breakdown produce `report.json`.
Emulator failure leaves diagnostics and exits nonzero without a success report.

The report records SHA-256 input/ELF/emulator identities, emulator version,
proposal block range/timestamps, elapsed native/emulator time, steps and
`zisk_emulator_operation_weighted_cost`. The weighted sum is base + main +
opcode + precompile + memory. It is neither setup-backed trace cells nor SP1
`prover_gas`; those fields remain null. Peak RSS is explicitly null unless
independently measured outside this tool. Preserve external measurement output
with the run and identify its scope (emulator versus complete host command).

## Measured CPU results (2026-10-23)

Both complete Mainnet proposals passed with the same proposal ELF, no input
rewriting, exact 32-byte native/guest output equality and zero unused public
slots. The fixture chain specs both specify Unzen activation at timestamp
1786021200; their block timestamps place them on opposite sides of that boundary.
No complete Hoodi fixture was available for this run.

| Metric | Proposal 23077, pre-Unzen | Proposal 41455, post-Unzen |
| --- | ---: | ---: |
| Block range | 9051439–9051630 | 12556056–12556247 |
| Complete witnesses | 192 | 192 |
| JSON bytes | 12,518,793 | 9,177,914 |
| Bincode bytes | 5,475,191 | 4,916,804 |
| Native validation seconds | 0.252050141 | 0.255646480 |
| Emulator + profiling seconds | 204.990716839 | 208.598006571 |
| Emulator steps | 2,521,392,431 | 2,575,662,701 |
| Weighted main cost | 171,454,685,308 | 175,145,063,668 |
| Weighted opcode cost | 97,062,306,114 | 98,105,355,448 |
| Weighted precompile cost | 1,147,903,558 | 1,187,430,647 |
| Weighted memory cost | 13,134,562,307 | 13,750,983,667 |
| Fixed base cost | 287,309,824 | 287,309,824 |
| Total weighted cost | 283,086,767,111 | 288,476,143,254 |
| Whole-command maximum RSS, KiB | 1,153,816 | 1,151,216 |

The emulator used about one CPU core per run. Runs overlapped on an 8-vCPU,
30-GiB Linux machine; these are single observations, not isolated timing
distributions. GNU `time -v` measured the complete benchmark command and its
children: wall times 205.34/208.95 seconds and exit status 0. The RSS scope is
that command tree, not a guest-only memory measurement; the generic JSON report
deliberately leaves its unmeasured RSS fields null. No proving keys were loaded,
no proof was generated and no setup-backed cell count was obtained.

Identities (SHA-256, full JSON fixture names are recorded in each run report):

- Proposal 23077 JSON: `5ce469871fdae792da91666f98424cb128de0272e9017d12d757648cbc4b4fbc`.
- Proposal 41455 JSON: `20c76ab78f7bde77612f28d82f734912247ee905fd682de0568d9013053cc575`.
- Proposal ELF: `ebd2bb9aa3e923452f0da4e643378ef26eb31fd892b41ef6da533cba76cf6193`.
- Emulator: `e43535727c3b1fa6bae565400b0b07e8ba515b0728494bc26ef9e862595cd5f6`,
  `ziskemu 1.3.2-alpha (1e22630 2026-10-09T21:09:05.902797033Z)`.
- Compiler archive: `21a895087d34491fbf9047d83c7ae9703d7e8b2599601303ce5d9f6411ed80ca`,
  verified before extraction, reports `rustc 1.99.0-dev`.
- SDK archive: `abb779427f6c6924c7482f943356f60c6577499432bb806af349669b2a729bd7`,
  verified before extraction; CPU tools only were executed.

Native/guest public digests are respectively
`5fe8c79b642e3068acf1a7fda9e4391ab27d0e85854a9da43ac6945d419bf8a6` and
`459cc28be1f717d59ccfa76de0fe610032429932637f264f5d808fe89bf02941`.
Independent artifact checks recomputed framing, padding, binary identities,
digests and CSV component sums. A real 100-step probe of proposal 41455 failed
with `EmulationNoCompleted`, returned nonzero and produced no success report.

Validation passed: focused Rust tests (71), the independent native C allocator
test (1), focused Clippy with `-D warnings`, workspace/guest formatting, both
guest target builds and the two complete CPU runs. Independent source review and
behavioral verification found no remaining blocker for this experimental lane.
Full-workspace Clippy was attempted but stopped in the existing
`risc0-circuit-recursion 4.0.4` build script when its external artifact download
returned HTTP 400; that lane is not claimed as passing.

The checked-in proposal 23077 fixture predates Unzen. Report fixture provenance
and the actual fork separately; that fixture alone cannot establish current-fork
compatibility. Saved external fixtures may be mounted read-only, without copying
machine-specific paths into source or committed reports.

## Aggregation boundary

`zisk-shasta-aggregation` requires `--features aggregation`, `ZISK_SETUP_VK` and
`ZISK_PROPOSAL_VK`. Each key is four comma-separated decimal u64 limbs from
authoritative setup/program artifacts. They are embedded during compilation;
missing, malformed, zero or noncanonical keys fail the build. No demo keys are
provided and keys are never accepted alongside untrusted input proofs.

The input is bincode 1.3.3 `ZiskAggregationInput`: full uncompressed proposal
proof word vectors, matching carry data and the zero ZK prover address. The
adapter accepts only `[minimal=0, n_publics=69, leaf=1, program_vk(4), publics(64),
proof..., appended_setup_vk(4), hash_tag]`. Folded and compressed children are
deliberately unsupported. It calls the real pinned `verify_zisk_proof` for each
child and binds the digest from those verified proof words to the shared carry
hash and continuity checks in `aggregate_shasta_zk_with_verifier`.

The experimental final program ID is the four expected proposal VK limbs
encoded little-endian into B256. There is no on-chain ZisK verifier contract
claim for this convention. Compilation with synthetic test keys establishes
only compilation; genuine child proof verification and aggregation execution
remain unverified until real proof artifacts are available.

Focused adapter tests:

```bash
cargo test -p raiko2-primitives-shasta zisk::tests
cargo test -p zisk-bench

zisk_native_test_dir="$(mktemp -d)"
rustc --edition=2021 --test guests/zisk/tests/c_allocator.rs \
  -o "${zisk_native_test_dir}/c-allocator-tests"
"${zisk_native_test_dir}/c-allocator-tests"
```
