# Precompile Status for Shasta

> **Scope: Shasta only. This document has not been extended to Unzen.**
>
> Every table below describes the Shasta fork, which maps to Ethereum `SpecId::SHANGHAI`. Unzen
> maps to `SpecId::OSAKA` (see `ForkId::as_spec_id` in `crates/primitives/src/chain_spec.rs`), so
> its active precompile set is a **superset** of the one documented here. Because Unzen is the fork
> active on every live network, treat this file as a historical Shasta reference until the Unzen
> address set is verified against `revm-precompile` and added.

This document describes the precompile surface relevant to the `raiko2` Shasta proving path.

It answers three separate questions:

1. Which precompiles are active under the Shasta fork mapping?
2. Which active precompiles are routed through guest-specific crypto hooks?
3. Which code paths exist in `revm-precompile` but are not active for Shasta?

Use this file together with the regression tests in:

- the upstream `alethia-reth` `crates/evm/src/spec.rs` tests at the revision pinned in `Cargo.lock`
- `guests/risc0/src/crypto.rs`
- `guests/sp1/src/crypto.rs`

## Fork Mapping

For the current Shasta path, `TaikoSpecId::SHASTA` maps to Ethereum `SpecId::SHANGHAI`, and
`revm-precompile` maps `SHANGHAI` to the `BERLIN` precompile set.

This means the active address set is exactly:

- `0x01` `ECRECOVER`
- `0x02` `SHA256`
- `0x03` `RIPEMD160`
- `0x04` `IDENTITY`
- `0x05` `MODEXP`
- `0x06` `BN254_ADD`
- `0x07` `BN254_MUL`
- `0x08` `BN254_PAIRING`
- `0x09` `BLAKE2F`

Notably, Shasta does **not** activate:

- `0x0A` `KZG_POINT_EVALUATION` (`CANCUN`)
- `0x0B..0x11` `BLS12_*` (`PRAGUE`)
- `0x0100` `P256VERIFY` (`OSAKA`)

This is not a new regression introduced by the current dependency upgrade. The old Taiko-flavored
`revm` path also mapped Taiko fork-specific specs to the `BERLIN` precompile set.

## Active Precompiles by Backend

### RISC0

| Address | Precompile | Active in Shasta | Guest hook |
| --- | --- | --- | --- |
| `0x01` | `ECRECOVER` | Yes | Yes |
| `0x02` | `SHA256` | Yes | Yes |
| `0x03` | `RIPEMD160` | Yes | No |
| `0x04` | `IDENTITY` | Yes | N/A |
| `0x05` | `MODEXP` | Yes | No |
| `0x06` | `BN254_ADD` | Yes | No |
| `0x07` | `BN254_MUL` | Yes | No |
| `0x08` | `BN254_PAIRING` | Yes | No |
| `0x09` | `BLAKE2F` | Yes | No |

### SP1

| Address | Precompile | Active in Shasta | Guest hook |
| --- | --- | --- | --- |
| `0x01` | `ECRECOVER` | Yes | Yes |
| `0x02` | `SHA256` | Yes | Yes |
| `0x03` | `RIPEMD160` | Yes | No |
| `0x04` | `IDENTITY` | Yes | N/A |
| `0x05` | `MODEXP` | Yes | No |
| `0x06` | `BN254_ADD` | Yes | Yes |
| `0x07` | `BN254_MUL` | Yes | Yes |
| `0x08` | `BN254_PAIRING` | Yes | No |
| `0x09` | `BLAKE2F` | Yes | No |

## Fallback Backends

`revm-precompile` is active for the full Shasta address set above, but many operations still use
its default backend implementations instead of guest-specific hooks.

For the current build graph, `revm-precompile` is compiled with `std` only. Optional accelerated
backends such as `c-kzg`, `blst`, `secp256k1`, `gmp`, `p256-aws-lc-rs`, and `bn` are not enabled.

That means the current behavior is:

- `ECRECOVER`: active, but uses the guest hook instead of the crate default path.
- `SHA256`: active, but uses the guest hook instead of the crate default path.
- `RIPEMD160`: active, default backend.
- `MODEXP`: active, default backend.
- `BN254_ADD` / `BN254_MUL`:
  - `RISC0`: active, default backend.
  - `SP1`: active, guest hook.
- `BN254_PAIRING`: active, default backend.
- `BLAKE2F`: active, default backend.

## KZG Clarification

Shasta blob validation in `raiko2` does not rely on the EVM `0x0A` KZG point-evaluation precompile.

Instead, the proving path computes and verifies KZG commitments and proof-of-equivalence data
inside the guest/runtime utility code under `crates/primitives/src/blob/util.rs`.

So the fact that `0x0A` is not active under Shasta does **not** mean blob proof validation is
missing.

## What This Does Not Prove

This document and its regression tests prove:

- the active Shasta precompile address set;
- the guest hook coverage that `raiko2` intentionally installs today.

They do not prove that every active precompile is exercised by every proposal. Whether a given
proposal actually touches `MODEXP`, `BN254_PAIRING`, or `BLAKE2F` still depends on transaction
content and execution trace.
