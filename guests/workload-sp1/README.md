# SP1 proposal workload experiment

This standalone program evaluates the full Shasta proposal execution with the
`evm-gas-only` execution profile. It consumes the existing canonical `GuestInput`,
retains proposal, witness, transaction, state-root and EVM gas validation, and changes
only the legacy Unzen meter's admission cap. Legacy opcode accounting still runs;
the legacy 100M cutoff does not select the experiment's workload.

The production proposal program and checked-in ELF assets are separate. This program
does not activate the new pricing model on a node or select its admission limit.
The host computes new node-known pricing using the same explicit execution profile.

The independent workspace pins SP1 guest SDK/lib/primitives 6.3.0 to match the
standalone workload estimator. It reuses the production SP1 crypto and syscall
implementations. It is a new program identity: repeat admitted controls with this
ELF and keep old production-program observations separate.

## Build and record identities

With the documented SP1/RISC-V build tools installed, build only this program from
this directory, using the shared Cargo target configured by the operator:

```sh
CARGO_BUILD_JOBS=1 CARGO_INCREMENTAL=0 \
CC_riscv64im_succinct_zkvm_elf='riscv64-unknown-elf-gcc -specs=picolibc.specs' \
AR_riscv64im_succinct_zkvm_elf=riscv64-unknown-elf-ar \
cargo prove build --locked --ignore-rust-version \
  --binaries sp1-workload-proposal --output-directory /path/to/fresh/programs
```

The output directory must be reserved for this build. From the repository root,
record the resulting ELF and reviewed source snapshot:

```sh
~/.venv/bin/python bin/zkgas/bench/record_workload_program.py \
  --elf /path/to/fresh/programs/sp1-workload-proposal \
  --output /path/to/fresh/programs/program-manifest.json
```

The manifest records the exact ELF, locked SDK generation and source hashes. It is
a provenance declaration, not an attestation about arbitrary ELF bytes. Review the
source/build and qualify behavior using both admitted controls and a candidate
exceeding the old 100M budget. The harness freezes the manifest, requires its profile
and ELF hash to match the selected program, and rejects mixed-profile reports or
resume roots.

Use a new harness configuration with `execution_profile: "evm-gas-only"`, the built
ELF, and a `program_manifest` artifact. See
[the workload harness guide](../../bin/zkgas/bench/README.md). Do not overwrite old
campaigns or published fixtures. These execute-only observations do not certify a
production program, proving throughput or a particular new-model admission limit.
