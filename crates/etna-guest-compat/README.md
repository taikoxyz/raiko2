# Etna guest compatibility diagnostic

This pre-merge diagnostic pins `etna-core` at
`e6e186fe6a2e3912739db9b9cae1740e31064824` from taiko-mono #22398. Its guest
binaries use the existing `guests/risc0` and `guests/sp1` patch tables and
lockfiles. The production packages remain the default members of their respective
workspaces.

The new guest Cargo.lock contents change the inputs tracked by production ELF
provenance. This diagnostic branch leaves production ELF provenance stale.

The fixed suite checks the native Comet QC and body commitments, rejects changed
part sets, deployment domains and signatures, verifies that nil votes add no
block power, and confirms that exact 2/3 voting power fails. It also checks committee derivation and commitments, the
first H+2 handover in a stock validator-state chain, and hashes of recorded cut
preimages. The fixture exercises committee derivation from a supplied snapshot.
The journal is for compatibility diagnostics. Range composition, L1
authentication, EVM execution and proof generation are separate checks.

## Fixture sources

The following inputs are read from the pinned taiko-mono revision.

- `packages/taiko-client-rs/crates/etna-core/testdata/cometbft-v0.40/differential.json`
  (SHA-256 `37057f04a9fad283cf60234a9b937fa54e2ca24757cc7392095cc1f3a4abddea`).
  The two `.pb` files are the decoded hex `block_proto` and `commit_proto` of
  `own_nil_sig_at_unix_epoch`. The corresponding RPC header and block ID,
  part-set and powers fields supply the expected commitments. This is the stock
  CometBFT v0.40.0 fixture signed with the public test keys consisting of 32
  repeated 1 or 2 bytes. The same test keys sign the synthetic 2/3 control.
- `testdata/etna_vectors/vectors.json` (schema 7, SHA-256
  `a7f03bb67964c90c06211095603582e78872ccb5e1143d5e4760b8aadbf038b6`).
  `fixtures/vectors.json` selects the deployment domain of the stock header,
  the committee config, the five registry entries and their entries root,
  `committee_sets[3]`, `val_state_chains.chains[0]` (gen0, heights 0–8,
  first switch and its keys), the two cut leaves at heights 6 and 12, and
  the `g0_p0_h18` cut. All recorded verification values are preserved; unused
  private signing fields from the generator inputs are omitted. The diagnostic
  reads the public keys only.

`fixtures/journal.bin` is assembled from those recorded values. Its 684 bytes
contain the ASCII marker `raiko2-etna-guest-compat-v1` followed by NUL, the
48-byte Comet chain ID, then 19 raw 32-byte digests in order. The digests are
the Comet header, part set, validator set, committee config, registry entries,
committee set, committee record, nine validator states, two ordered cut leaves
and the interval cut hash. Its SHA-256 is
`e052c0bdd653e7cab3749799ca2017928fd3e8463998c81c041a93cdfa06964d`.
The runners compare computed bytes with the stock journal so an identical result
on all three platforms cannot excuse a shared discrepancy with the vectors.

## Verified result

On 2026-11-18, isolated locked release builds used the existing Raiko2 guest
patch tables. Both guests halted successfully and emitted the same 684 bytes
as native Rust 1.99.0 and the independently reconstructed stock journal above.
Changing the expected marker in an isolated copy caused the native test to
fail. The original inputs passed after the test recompiled.

| Guest SDK | Guest rustc | CPU execution | Raw ELF SHA-256 |
| --- | --- | --- | --- |
| RISC Zero 3.0.5, RV32 | 1.91.1-dev | 15,774,476 cycles, 17 segments | `6906d4eabfb02189ad8545f1e7fbfc72f3aea9c7fc53cf5c3bd79ddf39af5753` |
| SP1 6.8.1, RV64 | 1.93.0-dev | 4,247,290 cycles | `0b97b20613439477d5a9db9bee50b2d52a20435cdad5593311a74ad471d99e2b` |

The RISC Zero runner used `r0vm` 3.0.4. These ELF hashes and cycle counts
record the 2026-11-18 inputs, before the unused signing fields were removed.
Native re-execution after the cleanup still matches the pinned 684-byte journal.
The cleanup has not been remeasured in either guest. Production ELF validation,
complete Etna range verification, proof generation and L1 verification remain
outside this checkpoint.

## Entry points

Use a credential-free isolated build environment with the locked dependency
cache and SDKs already available. Hold the shared Etna build lock and retain
the disk reserve defined in the plan. Bind the existing shared cache as
`CARGO_TARGET_DIR`, put host Cargo and Rust 1.99.0 on `PATH`, and expose the
existing guest toolchain binary directories in the variables below. Run from
the source root.

```bash
set -e
: "${CARGO_TARGET_DIR:?set the shared target inside the isolated runner}"
: "${ETNA_RISC0_TOOLCHAIN_BIN:?set the isolated RISC Zero toolchain bin directory}"
: "${ETNA_SP1_TOOLCHAIN_BIN:?set the isolated SP1 toolchain bin directory}"
export CARGO_BUILD_JOBS=2
export CARGO_INCREMENTAL=0

(
  cd bin/etna-guest-compat || exit 1
  cargo build --release --locked --offline -p etna-guest-compat --features risc0,sp1
  cargo test --release --locked --offline -p etna-guest-compat --features risc0,sp1 \
    --test journal -- --test-threads=2
)

(
  cd guests/risc0 || exit 1
  RUSTC="$ETNA_RISC0_TOOLCHAIN_BIN/rustc" RUSTDOC="$ETNA_RISC0_TOOLCHAIN_BIN/rustdoc" \
    RISC0_FEATURE_bigint2=1 \
    CARGO_TARGET_RISCV32IM_RISC0_ZKVM_ELF_RUSTFLAGS='-C passes=lower-atomic -C link-arg=-Ttext=0x00200800 -C link-arg=--fatal-warnings -C panic=abort --cfg getrandom_backend="custom"' \
    cargo build --release --locked --offline --target riscv32im-risc0-zkvm-elf \
    -p risc0-etna-core-diagnostic --bin risc0-etna-core-diagnostic
)

(
  cd guests/sp1 || exit 1
  printf -v etna_sp1_flags '%s\037' -C passes=lower-atomic \
    -C link-arg=--image-base=2013265920 -C panic=abort --cfg 'getrandom_backend="custom"' \
    -C llvm-args=-misched-prera-direction=bottomup -C llvm-args=-misched-postra-direction=bottomup
  RUSTC="$ETNA_SP1_TOOLCHAIN_BIN/rustc" RUSTDOC="$ETNA_SP1_TOOLCHAIN_BIN/rustdoc" \
    CARGO_ENCODED_RUSTFLAGS="${etna_sp1_flags%$'\037'}" \
    cargo build --release --locked --offline --target riscv64im-succinct-zkvm-elf \
    -p sp1-etna-core-diagnostic --bin sp1-etna-core-diagnostic
)
```

Set `RISC0_SERVER_PATH` to the inspected `r0vm` binary in the isolated
environment, then run:

```sh
RISC0_FEATURE_bigint2=1 "$CARGO_TARGET_DIR/release/etna-guest-compat" risc0 \
  "$CARGO_TARGET_DIR/riscv32im-risc0-zkvm-elf/release/risc0-etna-core-diagnostic"
"$CARGO_TARGET_DIR/release/etna-guest-compat" sp1 \
  "$CARGO_TARGET_DIR/riscv64im-succinct-zkvm-elf/release/sp1-etna-core-diagnostic"
```

The RISC Zero path wraps the raw ELF with the SDK's V1 compatibility kernel.
Each executor checks a successful CPU exit and prints the actual guest bytes
after comparing them against the native journal.
