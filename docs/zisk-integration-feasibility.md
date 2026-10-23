# ZisK integration feasibility

Status: source investigation, 2026-10-23. No ZisK guest has been built or executed in
this investigation. This note records feasibility and the next measurements; it
does not enable a new production route.

## Scope and pinned sources

The objective is a third, independently implemented zkVM backend that reuses the
current complete Shasta proposal `GuestInput` and proposal/sub-proof aggregation
flow, and exposes a proving-work metric richer than execution steps.

- Raiko2 baseline: [`5fb9f9792e9901cd40c0fb6753ba2519598a9595`](https://github.com/taikoxyz/raiko2/commit/5fb9f9792e9901cd40c0fb6753ba2519598a9595), freshly fetched `origin/main`.
- ZisK release and `main`: [`v1.3.2-alpha`, `1e22630bb7e143c30705a1603ed87cfd3da8c6ec`](https://github.com/0xPolygonHermez/zisk/tree/1e22630bb7e143c30705a1603ed87cfd3da8c6ec).
- ZisK guest compiler: [`zisk-4.0.0`, `ae4928fa0d3c7e4be46e8e8db185ac07436c0aa9`](https://github.com/0xPolygonHermez/rust/tree/ae4928fa0d3c7e4be46e8e8db185ac07436c0aa9).

These pins describe this experiment. They are not a proposal for runtime SDK/ELF
version checks or new zkGas policy gates. Developer-controlled builds and releases
decide when dependencies and guest programs change.

## Findings

| Question | Source-level result | Not yet established |
| --- | --- | --- |
| Reuse the proposal input and execution logic? | Yes: reuse `GuestInput` and call `prove_shasta_proposal`; transport needs an adapter. | Cross-compilation and execution on real proposal fixtures. |
| Keep proposal proofs followed by an aggregation guest? | Yes: ZisK exposes an in-guest proof verifier; the existing carry-data and aggregation-hash helper can be reused. | Actual child-proof verification, output parity and aggregation cost. |
| Measure more than steps/cycles without proving? | Yes: profiler weighted cost, or setup-backed execution cost in trace cells; standalone execution also reports a per-AIR instance plan. | Wall time and suitability of each path for Taiko workloads. |
| Compile latest Raiko2 with the current official ZisK compiler? | Not directly: Raiko2 and its reth dependencies require Rust 1.95; ZisK's selected compiler is based on Rust 1.94. | A suitable newer ZisK compiler or a validated custom toolchain. |

## Proposal input and guest execution

The current SP1 and RISC0 proposal guests both deserialize
`raiko2_primitives_shasta::GuestInput`, call
`raiko2_guest_common::prove_shasta_proposal`, and commit the returned 32-byte
sub-proof input hash. The shared helper includes proposal/blob validation,
canonical block reconstruction, execution and checkpoint binding. A ZisK wrapper
can call this same implementation; it should not copy the execution loop or use
an execution-only shortcut in place of the complete proposal validation.

The data type is reusable, but the SDK default wire encoding differs:

- Raiko2 uses bincode 1.3.3 in the existing guest paths.
- ZisK `io::read<T>` / `ZiskStdin::write<T>` use bincode 2 standard encoding.
- ZisK raw stdin frames carry a little-endian 8-byte length and 8-byte alignment.

The smallest proposed adapter preserves the existing payload: host-side
`bincode::serialize(&guest_input)` followed by `ZiskStdin::write_slice`, and
guest-side `ziskos::io::read_slice()` followed by bincode 1.3.3 deserialization.
An existing RISC0-framed input file must not be passed through unchanged as ZisK
stdin. `commit_slice(hash.as_slice())` preserves the 32 digest bytes; ZisK exposes
64 public u32 slots, so the adapter reads the first eight slots as little-endian
u32 bytes and checks the unused slots against the agreed padding convention.

Other build work remains even though the protocol input does not need redesign:

- Add a ZisK guest wrapper and build lane for `riscv64ima-zisk-zkvm-elf`.
- Supply ZisK crypto hooks/patches where appropriate. Existing SP1/RISC0 patches
  are backend-specific and must not be copied as ZisK acceleration support.
- Verify the pure Rust KZG path: the current non-`succinct` target selection uses
  `kzg-rs`'s `standard` feature. This is source-level routing, not a claim that KZG
  already compiles or runs efficiently on ZisK.
- Measure allocator/input-memory requirements on full proposals, not only tiny
  samples. ZisK supports alternative allocators; choose from measurements.

Evidence: [Raiko2 proposal guest](../guests/sp1/src/shasta_proposal.rs),
[shared proposal and aggregation helpers](../crates/guest-common/src/lib.rs),
[ZisK guest I/O](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/ziskos/entrypoint/src/io.rs),
[ZisK stdin framing](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/common/src/io/stdin/zisk_stdin.rs).

## Proposal/sub-proof aggregation

ZisK exposes
`ziskos::zisklib::verify_zisk_proof(proof, expected_setup_vk, expected_program_vk)`.
Its upstream aggregation example proves two leaf executions and verifies both in
a second guest. This supports our existing two-guest architecture in principle.

The proposed integration keeps that architecture:

1. A proposal proof authenticates the existing 32-byte sub-proof input hash.
2. The aggregation input carries ZisK proof words and the existing
   `ProofCarryData` vector. Proof formats are backend-specific, as they already
   are for RISC0 receipts versus SP1 compressed proofs.
3. The aggregation guest verifies each child against the intended setup/program
   VKs, extracts its authenticated public output and binds it to the matching
   `hash_shasta_subproof_input(carry)`.
4. It calls the shared `aggregate_shasta_zk_with_verifier` logic for carry
   continuity, commitment construction and the final aggregation hash, then
   commits that hash as raw bytes.

Two details are essential to the compatibility claim:

- ZisK has a setup VK and a distinct leaf program VK, each represented by four
  u64 limbs. The aggregation guest needs an authoritative expected identity, for
  example generated build-time constants, rather than trusting keys supplied
  alongside arbitrary child proofs. The upstream example reads both from stdin
  explicitly for demonstration only. Define the ZisK program-ID-to-`B256`
  encoding once and use it for the final public hash; do not apply SP1's
  `sp1_contract_block_program_id` conversion to ZisK keys.
- A successful boolean proof verification alone does not compare the child's
  public output with the carry hash. Decode the public slots from the same
  verified proof, validate the format, and compare the recovered 32-byte hash.
  Host-supplied `block_inputs` alone are insufficient.

The in-guest verifier is ordinary verifier computation compiled into the ZisK
guest. Its cost must be measured; the existence of this API does not establish
SP1-like verification cost or justify borrowing our RISC0 aggregation estimate.

ZisK also has a separate Circom `Recurser`/`AggregateProofsRequest` path for folding
proofs with custom public-value merge circuits. That is a different implementation
choice, requiring Taiko merge logic in Circom, and is not needed to establish
compatibility with our existing Rust aggregation helper.

Evidence: [in-guest verifier](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/ziskos/entrypoint/src/zisklib/lib/zisk_verifier.rs),
[upstream aggregation host](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/examples/aggregation/host/src/main.rs),
[public layout/parser](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/verifier/src/verifier.rs),
[recurser builder](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/sdk/src/recurser.rs).

## Proving-work metrics, beyond steps

SP1's reported `prover_gas` and ZisK's cost are different vendor-specific units.
ZisK cost can serve the same purpose in workload modelling, but values must stay
separate; neither is Ethereum transaction gas or an actual monetary quote.

There are three distinct measurement paths in the inspected release:

| Path | Proving keys needed | Output beyond steps | Interpretation |
| --- | --- | --- | --- |
| Emulator profiling (`ziskemu -X`, or SDK `run` with profiling) | No | Weighted opcode, memory, precompile and main costs, plus a fixed base term | Operation-weighted model; useful for opcode/precompile calibration. |
| SDK `execute_only()` / CLI `cargo-zisk execute` | No | Planned instance counts per AIR (proof circuit) | Concrete workload composition; `get_execution_cost()` is explicitly `None`, not zero. |
| Full embedded client `execute`, without `prove` | Yes | `get_execution_cost() = Some(cells)` and categorized costs in executor statistics | Setup-aware planned base-proof trace cells, including circuit padding and fixed tables. |

For the setup-backed path, `ProofmanAdapter::compute_costs` derives each AIR's
weight as `2^n_bits * sum(non-constant column widths)`, multiplies by planned
instance count and includes the fixed virtual tables. The total is the sum of
main, opcode, memory, precompile, table and other categories. It therefore captures
workload differences that a single cycle/step count misses.

The lightweight profiler instead uses its own operation and memory weights:
`base + main + opcode + precompile + memory`. Those numbers must not be silently
mixed with setup-backed cells, even when both are displayed as "cost". Store a
metric kind, SDK/ELF identity and raw breakdown with every sample. A future
standalone-plan-to-cells adapter could use exported setup metadata, but that would
be our adapter to validate, not a native cost returned by `execute_only()`.

Both paths execute the guest; source inspection does not establish a fast native
gas estimator equivalent to SP1's direct estimator. Record setup, execution,
count-and-plan and total wall time separately. These cost metrics also do not
directly include the entire recursive proving/SNARK wrapping pipeline or predict
hardware-dependent proving time without calibration.

Evidence: [SDK execute-only client](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/sdk/src/embedded/execute_only.rs),
[execution result and optional cost](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/prover-backend/src/output.rs),
[setup-based cost formula](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/executor/src/adapters.rs),
[profiler cost report](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/emulator/src/stats/stats.rs),
[operation weights](https://github.com/0xPolygonHermez/zisk/blob/1e22630bb7e143c30705a1603ed87cfd3da8c6ec/emulator/src/emu_costs.rs).

## Compiler obstacle and next verification

The CLI selects the newest `zisk-4.x.y` toolchain tag, not the repository's older
GitHub `releases/latest` pointer. At investigation time that selects
`zisk-4.0.0`; its [`src/version`](https://github.com/0xPolygonHermez/rust/blob/ae4928fa0d3c7e4be46e8e8db185ac07436c0aa9/src/version)
is `1.94.0`. Current Raiko2, alethia-reth and reth declare Rust 1.95. Installing the
current official ZisK toolchain alone would not resolve that requirement.

The preferred next step is to obtain a ZisK-compatible Rust 1.95+ compiler, or
validate a custom build of its toolchain. Do not downgrade the shared reth pins or
ignore MSRV checks and call that current-main compatibility.

After resolving the compiler requirement, the smallest useful verification is:

1. Build only the proposal wrapper and execute saved, complete Mainnet and Hoodi
   proposal fixtures through the shared validation path. Compare the exact
   32-byte public output with the native result; record input size, elapsed time
   and memory.
2. For the same ELF/inputs, capture profiler weighted cost, standalone AIR plans
   and setup-backed cells as separate metrics. Check reproducibility and inspect
   which operations dominate. No proving is needed for these cost samples.
3. Generate genuine proposal child proofs and run the aggregation guest with
   1/2/5 children, then check wrong program/setup identity, mutated proof/output,
   mismatched carry and reversed continuity. Compare its final hash with the
   existing shared helper. Execute-only or mocked verification cannot establish
   child-proof compatibility.
4. Only after these measurements add the normal `ProverBackend` ELF selector,
   `Prover` implementation and public route/config wiring. Keep experimental cost
   collection independent of any decision to update a production zkGas table.

This investigation checked source and release metadata only. There is no local
ZisK toolchain/CLI available, and no proposal, child proof, aggregation or timing
test has run. The machine also had approximately 5 GiB free at inspection time;
large proving artifacts should not be downloaded as an incidental research step.
