# Precompile Status for Unzen

This document describes the precompile surface relevant to the current `raiko2` proving path. Every
Taiko network runs the Unzen fork.

It answers three separate questions:

1. Which precompiles are active under the Unzen fork mapping?
2. Which active precompiles are routed through guest-specific crypto hooks?
3. Where do the RISC0 and SP1 guests differ?

Use this file together with:

- the upstream `alethia-reth` `crates/evm/src/spec.rs` tests at the revision pinned in `Cargo.lock`
- `guests/risc0/src/crypto.rs`
- `guests/sp1/src/crypto.rs`

## Fork Mapping

`raiko2` maps `TaikoFork::Unzen` to Ethereum `SpecId::OSAKA`
(`crates/primitives/src/chain_spec.rs:376`). Every other Taiko fork, including Shasta, falls
through to `SpecId::SHANGHAI` (`:377`).

This document describes `revm-precompile` version `34.0.0`, which is what both guests pin
(`guests/sp1/Cargo.toml`, `guests/risc0/Cargo.toml`). The workspace lockfile also contains
`41.0.0` for host-side crates; that is not the version the guests compile against.

In `34.0.0`, `SHANGHAI` collapses to `PrecompileSpecId::BERLIN` while `OSAKA` maps to
`PrecompileSpecId::OSAKA`, composed as:

```
osaka   = prague + modexp::OSAKA + secp256r1::P256VERIFY_OSAKA
prague  = cancun + bls12_381::precompiles()
cancun  = berlin + kzg_point_evaluation::POINT_EVALUATION
berlin  = istanbul + modexp::BERLIN
istanbul = byzantium + bn254 repricing + blake2::FUN
```

Unzen therefore activates nine addresses that the previous `SHANGHAI` mapping never had: `0x0A`,
`0x0B` through `0x11`, and `0x100`.

## Active Precompiles and Guest Hook Coverage

Every address below is active under Unzen. The `Crypto` trait exposes 17 overridable methods;
`Risc0GuestCrypto` overrides 6 and `Sp1GuestCrypto` overrides 4.

| Address | Precompile | Introduced | RISC0 hook | SP1 hook |
| --- | --- | --- | --- | --- |
| `0x01` | `ECRECOVER` | Homestead | Yes | Yes |
| `0x02` | `SHA256` | Homestead | Yes | Yes |
| `0x03` | `RIPEMD160` | Homestead | No | No |
| `0x04` | `IDENTITY` | Homestead | No hook exists | No hook exists |
| `0x05` | `MODEXP` | Byzantium, repriced by Berlin and Osaka | Yes | No |
| `0x06` | `BN254_ADD` | Byzantium | Yes | Yes |
| `0x07` | `BN254_MUL` | Byzantium | Yes | Yes |
| `0x08` | `BN254_PAIRING` | Byzantium | No | No |
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

## Two Findings

**The eight precompiles newly activated by Unzen have no guest hook in either backend.** `0x0A` and
`0x0B` through `0x11` run entirely on default backend implementations inside the zkVM.

**RISC0 and SP1 diverge on two addresses.** RISC0 overrides `modexp` and
`secp256r1_verify_signature`; SP1 overrides neither. So `0x05` `MODEXP` and the newly active `0x100`
`P256VERIFY` run unaccelerated under SP1 but accelerated under RISC0.

## Backend Selection

Both guests build `revm-precompile` with `default-features = false, features = ["bn"]`. That
selects the `bn` crate for BN254 and enables neither `blst` nor `c-kzg`.

Consequences:

- BN254 operations use the `bn` backend, further overridden by guest hooks at `0x06` and `0x07`.
- BLS12-381 (`0x0B` through `0x11`) falls back to the pure-Rust `ark-bls12-381` implementation.
- KZG (`0x0A`) falls back to the pure-Rust arkworks implementation rather than `c-kzg`.

The SP1 hooks are not syscall shims in the way the RISC0 hooks are. `Sp1GuestCrypto::sha256`
delegates to `sha2::Digest`, `secp256k1_ecrecover` to `k256`, and the BN254 operations to hand
written `BigUint` arithmetic. These depend on SP1's patched crate graph for acceleration rather than
on direct precompile calls.

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
