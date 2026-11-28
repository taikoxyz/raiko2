# Etna execution candidate compatibility checkpoint

This is an isolated dependency/source preflight, not execution integration or
proof acceptance. The production baseline is Raiko2 `a1fb48ba3ffd1de7b9dfef1939cf66ff58366d08`.
No production manifest, lock, execution code, SDK or generated guest artifact
changed. RISC0 artifact recovery/builds have lower priority than native/SP1.

## Exact diagnostic inputs

| Input | Checked identity |
| --- | --- |
| Alethia candidate | [#248 at ad584325](https://github.com/taikoxyz/alethia-reth/tree/ad584325c6b1e15f3c163fb25e71c04a78b194e0) |
| Candidate OP-reth | [op-v2.7.0-1 at ed91041f](https://github.com/ethereum-optimism/reth/tree/ed91041f36b973eda3521ff90b3a25165087c133) |
| Canonical Alethia main at refresh | `fce87eff1d7b720852468e566a6df686b5487dbc`, without Etna support |
| Host toolchain | Rust 1.95.0 from the production toolchain file |
| SP1 dependency graph | SDK/guest 6.8.1, existing crypto patch commits retained |

The candidate remains unmerged. Its temporary adoption below is diagnostic;
production adoption still requires a reviewed compatible Alethia main commit.
The source copy omits large ELF assets and is not a full guest-build fixture.

## Dependency resolution

Changing only Alethia pins fails real Cargo resolution: OP-reth 2.7 requires
Alloy consensus at least 2.5.0 while guest-common requires exactly 2.4.2.
The temporary source copy makes only these compatibility edits:

- Align all direct Alethia edges, including guest-common and xtask, to the same
  candidate revision. Preserve disabled defaults and existing features.
- Align the direct OP-reth edges to the candidate's exact URL/tag identity.
  The diagnostic lock binds that tag to the full commit above. Cargo source
  identity includes the query; a separate rev spelling is not equivalent for
  types crossing crate boundaries.
- Change guest-common's exact Alloy consensus to 2.5.0 and hardforks to 0.4.9;
  align the workspace hardfork minimum. Keep Alloy EIPs 1.8.3 for BlobCoder.
- Change workspace reth-primitives-traits from 0.7.1 to the candidate's 0.8.1
  requirement; the diagnostic resolver selected 0.8.2.

The native graph resolves 1,173 packages. Its 35 added and 35 removed package
identities are confined to Alethia, Reth and Alloy packages. A legacy Reth/Alloy
closure remains for the existing client dependency; repeated package names
alone are not evidence of mismatched types at an execution call site.

Independent runs of the following checks passed against the frozen copy with
read-only source/caches, no external network, the existing shared target and a
12 GiB disk reserve:

```sh
cargo metadata --locked --offline --format-version 1
cargo metadata --locked --offline --format-version 1 \
  --manifest-path guests/sp1/Cargo.toml \
  --filter-platform riscv64im-succinct-zkvm-elf
cargo metadata --locked --offline --format-version 1 \
  --manifest-path guests/sp1/Cargo.toml \
  --filter-platform riscv64im-succinct-zkvm-elf --features bench
```

SP1 metadata used the existing succinct compiler for target configuration.
These checks resolve dependency graphs; they do not compile guest code. The
14 resolved SP1 SDK/guest and patched crypto package identities match the
production SP1 lock. No RISC0 build or artifact download was attempted.

The independent graph audit also checks the actual consumers: stateless,
guest-common, pipeline and provider share the candidate native execution type
identities. SP1's normal target closure, excluding host proc-macros, contains 427 packages
in both modes;
bench adds only the guest/guest-common feature. The legacy client dependency
still brings its older Alethia/Reth/REVM/Alloy closure, as in production. The
SP1 graph also retains tokio, c-kzg and libz-sys paths; graph resolution does not
validate their target behavior. No host SP1 SDK is reachable from the guest.
Do not treat a successful metadata check as a portability or link test.

## Native compiler boundary

The isolated command `cargo check --locked --offline -p raiko2-stateless --lib`
with the pinned Rust 1.95.0 exits 101 before compiling: the five candidate
Alethia packages declare Rust 1.99. No replay result or API compilation result
was produced. A native 1.99 diagnostic can be done separately; changing the
production compiler requires its normal compatibility validation.

This does not establish a necessary SP1 SDK upgrade. Both existing SP1 build
paths already pass `--ignore-rust-version`; Alloy consensus's declared minimum
also did not rise between the two selected versions. Actual compilation with
the pinned guest SDK must identify any real language/library incompatibility.
Do not add a new MSRV bypass to the native production build.

## Legacy API boundary

Raiko2 `crates/primitives/src/chain_spec.rs` still calls
`TaikoDevnetConfigExt::clone_with_devnet_unzen_timestamp`. The exact Alethia
candidate removes that method and exposes `clone_with_devnet_etna_timestamp`
instead. Source inspection establishes a missing API; the native check above
stops before Rust method resolution and does not independently compile it.

The old method synchronizes Unzen and Cancun/Prague/Osaka activation and
regenerates the cached genesis header. The new method selects Etna and rebuilds
from a different fork policy. Renaming the call would change legacy behavior.
Preserve the old helper upstream or obtain an equivalent upstream-supported
legacy path before production adoption; do not duplicate those genesis rules
inside Raiko2 or drop Unzen compatibility silently.

## Reuse boundary and next steps

At the exact candidate OP-reth revision, the ordinary supplied-block path still
calls `Executor::execute` then `BasicBlockExecutor::execute_one`, executing each
committed transaction with propagated errors. The state-hook path dispatches
to the trait override; the candidate makes that override strict for Etna too.
This was independently inspected source, not a run of its regression tests.

Continue through the existing validator, signer recovery, witness DB, trie and
post-state-root check once dependencies are compatible. Keep legacy anchor and
receipt assertions on Unzen; select Etna's rules from the trusted chain spec.
Candidate built-in profiles leave Etna disabled, so successful compilation alone
will not establish trusted activation. Then connect the already authenticated
DA/range projection to real per-block witness replay. QC, FI, anchor authority,
final public-output framing and proof acceptance retain their separate gates.
