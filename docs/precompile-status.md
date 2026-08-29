# Precompile Status for Unzen

This document describes the precompile surface relevant to the current `raiko2` proving path. Every
Taiko network runs the Unzen fork.

It answers three separate questions:

1. Which precompiles are active under the Unzen fork mapping?
2. Which active precompiles are routed through guest-specific crypto hooks?
3. Where do the RISC0 and SP1 guests differ?

Use this file together with:

- the upstream `alethia-reth` `crates/evm/src/spec.rs` tests at rev `6c4d199`, which `Cargo.toml`
  pins
- `guests/risc0/src/crypto.rs`
- `guests/sp1/src/crypto.rs`

## Fork Mapping

The mapping that selects the guest precompile set is `TaikoSpecId::into_eth_spec` in `alethia-reth`
`crates/evm/src/spec.rs`, at the revision raiko2 pins: `UNZEN` maps to `SpecId::OSAKA`, while
`GENESIS`, `ONTAKE`, `PACAYA`, and `SHASTA` all map to `SpecId::SHANGHAI`.

`raiko2` mirrors this host-side in `ForkId::as_spec_id`
(`crates/primitives/src/chain_spec.rs:376-377`), where `TaikoFork::Unzen` maps to `SpecId::OSAKA`
and every other Taiko fork falls through to `SpecId::SHANGHAI`. The two agree, but the mirror has
no callers outside that file: it feeds chain-spec validation and preflight cache identity, not guest
execution. Editing it does not change what the guest runs.

This document describes `revm-precompile` version `34.0.0`, which is what both guests pin
(`guests/sp1/Cargo.toml`, `guests/risc0/Cargo.toml`).

A second copy, `revm-precompile 41.0.0`, is also compiled into both guests. It arrives
unconditionally through `taiko-client-protocol` -> `alethia-reth-chainspec 1.3.0` ->
`reth-chainspec 2.4.0` -> `alloy-evm` -> `revm 41.0.0`, and it appears in both guest lockfiles.
The guests are excluded from the root workspace, so they carry their own lockfiles and the root
lockfile does not govern them. The `41.0.0` copy is linked but sits off the execution path: the
guest EVM runs `alethia-reth-block` and `alethia-reth-evm` at the pinned revision ->
`reth-revm 2.0.0` -> `revm 38.0.0` -> `revm-precompile 34.0.0`. That is also the copy
`install_crypto` registers hooks into, so guest hooks affect only `34.0.0`. The `41.0.0` copy has
its own `install_crypto` global, which is never populated.

In `34.0.0`, `SHANGHAI` collapses to `PrecompileSpecId::BERLIN` while `OSAKA` maps to
`PrecompileSpecId::OSAKA`, composed as:

```
osaka   = prague + modexp::OSAKA + secp256r1::P256VERIFY_OSAKA
prague  = cancun + bls12_381::precompiles()
cancun  = berlin + kzg_point_evaluation::POINT_EVALUATION
berlin  = istanbul + modexp::BERLIN
istanbul = byzantium + bn254 repricing + blake2::FUN
byzantium = homestead + modexp::BYZANTIUM + bn254::{add,mul,pair}::BYZANTIUM
homestead = ECRECOVER + SHA256 + RIPEMD160 + IDENTITY
```

`revm` names its floor set `homestead`, though `0x01` through `0x04` were live from Frontier; the
crate folds Frontier through Spurious Dragon into that set.

Unzen therefore activates nine addresses that the previous `SHANGHAI` mapping never had: `0x0A`,
`0x0B` through `0x11`, and `0x100`.

## Active Precompiles and Guest Hook Coverage

Every address below is active under Unzen. The `Crypto` trait exposes 17 overridable methods;
`Risc0GuestCrypto` overrides 6 and `Sp1GuestCrypto` overrides 4.

| Address | Precompile | Introduced | RISC0 hook | SP1 hook |
| --- | --- | --- | --- | --- |
| `0x01` | `ECRECOVER` | Frontier | Yes | Yes |
| `0x02` | `SHA256` | Frontier | Yes | Yes |
| `0x03` | `RIPEMD160` | Frontier | No | No |
| `0x04` | `IDENTITY` | Frontier | No hook exists | No hook exists |
| `0x05` | `MODEXP` | Byzantium, repriced by Berlin and Osaka | Yes | No |
| `0x06` | `BN254_ADD` | Byzantium, repriced by Istanbul | Yes | Yes |
| `0x07` | `BN254_MUL` | Byzantium, repriced by Istanbul | Yes | Yes |
| `0x08` | `BN254_PAIRING` | Byzantium, repriced by Istanbul | No | No |
| `0x09` | `BLAKE2F` | Istanbul | No | No |
| `0x0A` | `KZG_POINT_EVALUATION` | Cancun | No | No |
| `0x0B` | `BLS12_381_G1_ADD` | Prague | No | No |
| `0x0C` | `BLS12_381_G1_MSM` | Prague | No | No |
| `0x0D` | `BLS12_381_G2_ADD` | Prague | No | No |
| `0x0E` | `BLS12_381_G2_MSM` | Prague | No | No |
| `0x0F` | `BLS12_381_PAIRING` | Prague | No | No |
| `0x10` | `BLS12_381_MAP_FP_TO_G1` | Prague | No | No |
| `0x11` | `BLS12_381_MAP_FP2_TO_G2` | Prague | No | No |
| `0x100` | `P256VERIFY` | Osaka | Yes | No |

`0x04` `IDENTITY` is a byte copy with no cryptographic work, so the `Crypto` trait defines no hook
for it.

## Coverage Gaps

**Eight of the nine precompiles newly activated by Unzen have no guest hook in either backend.**
`0x0A` and `0x0B` through `0x11` run their core operation on default backend implementations inside
the zkVM. One nuance at `0x0A`: its versioned-hash step calls `crypto().sha256`, which both guests
do hook, so only its `verify_kzg_proof` core is unaccelerated. The ninth newly active address,
`0x100`, is covered by RISC0 but not SP1.

**RISC0 and SP1 diverge on two addresses.** RISC0 overrides `modexp` and
`secp256r1_verify_signature`; SP1 overrides neither. So `0x05` `MODEXP` and the newly active `0x100`
`P256VERIFY` run unaccelerated under SP1 but accelerated under RISC0.

## Backend Selection

Both guests build `revm-precompile` with `default-features = false, features = ["bn"]`. That
selects the `bn` crate for BN254 and enables neither `blst` nor `c-kzg`.

Consequences:

- Both guests hook `0x06` and `0x07`, so the `bn` backend (the `substrate-bn` crate) governs only
  `0x08` `BN254_PAIRING`.
- BLS12-381 (`0x0B` through `0x11`) falls back to the pure-Rust `ark-bls12-381` implementation.
- KZG (`0x0A`) falls back to the pure-Rust arkworks implementation rather than `c-kzg`.

The SP1 hooks are not syscall shims in the way the RISC0 hooks are, and they do not all benefit
from SP1's patched crate graph. `Sp1GuestCrypto::sha256` delegates to `sha2` and
`secp256k1_ecrecover` to `k256`; `guests/sp1/Cargo.toml` patches both to SP1 forks, so those two are
accelerated.

The BN254 hooks at `0x06` and `0x07` are not. They are hand-written `BigUint` arithmetic over
`num-bigint 0.4.6` (`guests/sp1/Cargo.toml`), which is absent from that `[patch.crates-io]` block.
The result is an inversion worth measuring: the two BN254 operations SP1 hooks run on unpatched
arithmetic, while `0x08` `BN254_PAIRING`, which SP1 does NOT hook, falls through to the `bn`
backend (`substrate-bn`) that the patch block DOES cover.

## KZG Clarification

Blob validation in `raiko2` does not use the EVM `0x0A` KZG point-evaluation precompile, even
though Unzen activates it.

The proving path computes and verifies KZG commitments and proof-of-equivalence data inside the
guest and runtime utility code under `crates/primitives/src/blob/util.rs`.

The absence of a guest hook for `0x0A` therefore says nothing about blob proof validation, which
does not route through that precompile.

## What This Document Does Not Establish

This document establishes the active Unzen precompile address set and the guest hook coverage
`raiko2` installs today.

It does not establish which of these precompiles real proposals actually reach. Whether Taiko
execution ever touches `0x0A`, `0x0B` through `0x11`, `0x08`, or `0x09` depends on transaction
content and execution trace, and has not been traced. Treat the newly activated addresses as a
correctness surface that is now reachable in principle, not as a measured hot path.
