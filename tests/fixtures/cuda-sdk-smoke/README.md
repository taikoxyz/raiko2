# Tiny CUDA SDK smoke fixtures

These fixed programs check local SDK execution and CUDA proof generation plus local verification.
They do not validate Taiko proposal or aggregation correctness, throughput, memory requirements,
multi-GPU scheduling, or production readiness. Inputs are embedded in `guest-launcher`; optional
`--input` files must match the corresponding fixed case.

From the repository root, run the CPU fixture prechecks:

```sh
cargo run -r -p guest-launcher -- --stage sdk-smoke --proof-type sp1 --mode execute
cargo run -r -p guest-launcher -- --stage sdk-smoke --proof-type risc0 --mode execute
```

With the NVIDIA driver, CUDA toolkit and backend GPU prerequisites prepared, run genuine proofs:

```sh
cargo run -r -p guest-launcher --features cuda -- --stage sdk-smoke --proof-type sp1 --mode prove --json-out sp1-smoke-report.json --output sp1-smoke-proof.json
cargo run -r -p guest-launcher --features cuda -- --stage sdk-smoke --proof-type risc0 --mode prove --json-out risc0-smoke-report.json --output risc0-smoke-proof.json
```

SP1 needs the matching `sp1-gpu-server` executable expected by SDK 6.8.1. The SDK manages its
normal server process; this launcher adds no service or scheduler. SP1 produces a compressed proof;
RISC0 produces a succinct receipt. Both verify locally against the program identity and check exact
public output. Proving requires the `cuda` feature and fails without it; CPU proving, network,
mock, development receipts and outer Plonk/Groth16 wrapping are excluded. Execute mode reports
`accelerator: cpu` and `verified: false`; it is a fixture check, not GPU or proof verification.
The JSON report is printed to stdout and optionally saved with `--json-out`. `--output` saves the
SDK proof/receipt JSON and is supported only for prove mode.

## SP1

`sp1-add.json` (125 bytes including newline) is parsed as `OpcodeLabInput` and passed via
`SP1Stdin::write`. It executes `PUSH1 1; PUSH1 2; ADD; STOP`, yielding accumulator 3.
The existing committed `crates/guests/elf/sp1_opcode_lab.elf` is embedded directly, without a
second ELF copy or rebuild. Its source is `guests/sp1/src/opcode_lab.rs` and
`guests/sp1/src/opcode_lab_impl.rs`; build provenance is `crates/guests/elf/sp1.provenance.json`.
The source and ELF follow the repository MIT/Apache-2.0 license.

- SDK: SP1 6.8.1.
- ELF SHA-256: `982d9347a651cd149ec57a24656fe288be6f5a6cfc91e21098905f065ea292c2`.
- Expected raw public values: `0x5122fb7425a8c48308cb691e90bf72c00d68db638f065ef886d5909adb3205df`.

The digest is Keccak-256 of UTF-8 `cuda-smoke-add`, opcode byte `01`, then little-endian
64-bit target count 1, target raw gas 3 and accumulator 3. It is raw committed digest bytes,
not a serialized hex string.

## RISC0

`risc0-loop.bin` and `risc0-loop.s` are unmodified copies of the files at
`risc0/zkvm/examples/loop.{bin,s}` in upstream
[`risc0/risc0` commit ba889fc049830b777de63fddbeea5f9776618d7d](https://github.com/risc0/risc0/tree/ba889fc049830b777de63fddbeea5f9776618d7d/risc0/zkvm/examples),
packaged in `risc0-zkvm` 3.0.5. The assembly preserves its Apache-2.0 copyright header;
`LICENSE-APACHE` contains the license text. The upstream package has no additional NOTICE file.

- `risc0-loop.bin`: 656 bytes; SHA-256 `cc7bdb6579ecf77a6bbaacb886953ac87eb36851609450aec20242395bbe67af`.
- `risc0-loop.s`: 2236 bytes; SHA-256 `882f89d11cdba9b2add40f445d38d2fb2b0aa4aff564c08ab8a869ca4d2b21a4`.
- SDK: RISC0 3.0.5; program wrapper `risc0-binfmt` 3.0.4 and
  `risc0-zkos-v1compat` 2.2.2 (`V1COMPAT_ELF`).
- Input: `risc0-loop.json`, `{"iterations":0}`; four zero bytes via `write_slice`.
- Expected exit: `Halted(0)`; expected journal: empty (`0x`).

The launcher wraps the raw ELF with `ProgramBinary::new(..., V1COMPAT_ELF).encode()` and computes
the wrapped program image ID. Proving and verification explicitly disable development mode;
only a genuine succinct receipt is accepted, regardless of `RISC0_DEV_MODE`.
