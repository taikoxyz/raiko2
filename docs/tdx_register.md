# Registering a TDX prover on-chain

`cargo run -p xtask -- register-tdx` admits a Nethermind-TDX prover to the on-chain
`AzureTdxVerifier` instance registry so its proofs verify on Taiko. This page covers when to
use each flag, what each one does, and the common day-2 flows (key rotation, new image
rollout).

Throughout this doc, `TdxVerifier` refers to the proxy/role name in operator runbooks and
chain-spec entries; the deployed implementation contract is `AzureTdxVerifier`
([`taiko-mono/.../AzureTdxVerifier.sol`](https://github.com/taikoxyz/taiko-mono/blob/main/packages/protocol/contracts/layer1/verifiers/AzureTdxVerifier.sol)).
The xtask's `--verifier` flag and chain spec's `TDX` entry both point at the proxy address
that delegates to this implementation.

For the broader pipeline (image build, VM deploy, smart-contract deploy) see
[`taiko-mono/.../tdx_deployment.md`](https://github.com/taikoxyz/taiko-mono/blob/main/packages/protocol/docs/tdx_deployment.md).
For the runtime architecture (what runs inside the image and how the pieces talk) see
[`tdx_architecture.md`](./tdx_architecture.md).

## What the command does

The xtask fetches the prover's TDX bootstrap data — either from the running `reth-tdx`
HTTP API (`GET <url>/bootstrap`, served from inside the TDX VM) or from
`~/.config/reth-tdx/bootstrap.json` on the local filesystem — then sends up to two
transactions to the verifier (i.e. the `AzureTdxVerifier` behind the proxy):

| Flag | Transaction | Permission | Purpose |
|------|-------------|------------|---------|
| `--trust` | `setTrustedParams(index, params)` | **Owner-only** | Records the running image's hardware measurements (`mrSeam`, `mrTd`, `teeTcbSvn`, PCR digests). Any future `registerInstance` against this index will be admitted only if its quote matches these values. |
| `--register` | `registerInstance(trustedParamsIdx, attestation)` | Permissionless | Submits the Azure vTPM + Intel TDX DCAP attestation. The contract runs the full attestation flow on-chain (`AzureTDX.verify` → Automata DCAP → measurement equality vs. the trusted-params slot) and admits the bootstrap public key to the registry. |
| `--dry-run` | — | — | Print the transactions that would be sent and exit. |

If you pass both flags, `setTrustedParams` is sent first, then `registerInstance` reads the
same trusted-params slot. Both default to slot `0` (`--trusted-params-index`).

## When do I need `--trust`?

`--trust` writes the **policy** that future registrations are checked against. You only need
it when:

1. **First time the verifier is used.** Slot 0 starts empty; without it, `registerInstance`
   reverts with `TDX_INVALID_TRUSTED_PARAMS()`.
2. **Rolling out a new VM image.** Measurements (`mrTd`, `mrSeam`, PCRs) change every time
   you rebuild the image. Pick a fresh `--trusted-params-index` (e.g. `1`) and run with
   `--trust` again. Old instances stay valid against slot 0 until they expire, new instances
   register against slot 1.

You **don't** need `--trust` when:

- Re-registering the same image (the measurements haven't changed; the slot already holds
  the right policy).
- Registering a second VM running the same image (same measurements, same slot).
- Rotating the prover key (a fresh attestation is admitted to the registry by re-running the
  attestation flow; the policy slot is unchanged).

Because `--trust` calls an owner-only function, the `--private-key` you pass for it must be
the `TdxVerifier` owner. For `--register` alone, any funded account works.

## When do I need `--register`?

`--register` admits a specific TDX key (the prover's bootstrap `public_key`) to the registry.
You need it any time:

- Bringing up a new VM (its bootstrap key has never been seen on-chain).
- After the 365-day `INSTANCE_EXPIRY` on an existing key (the same key can re-register if it
  still attests successfully).
- After the prover's signing key rotates (e.g. rebuilt the image and the TDX-sealed key
  changed).

## Common flows

### 1. Trust + register (first-time or new image)

Requires the `TdxVerifier` **owner key** and a running VM. `--release-url`
cross-checks the VM's PCRs against the release's `measurements.json` before
broadcasting — use it to confirm the VM is running the expected image.

```bash
cargo run -p xtask -- register-tdx \
  --verifier 0x<TdxVerifier proxy>  \
  --rpc http://<L1 RPC>             \
  --private-key 0x<owner key>       \
  --reth-tdx-url http://<VM_IP>:8080 \
  --release-url https://github.com/NethermindEth/nethermind-tdx/releases/tag/<TAG> \
  --trust --register
```

> `--trusted-params-index` defaults to `0`. Pass `--trusted-params-index 1`
> when rolling out a new image while keeping the old policy in slot 0.

### 2. Register only (admit a new VM against an existing policy)

Use when the trusted-params slot already holds the right policy. **Any funded
account works** — the contract verifies the attestation autonomously. No
`--release-url` needed.

```bash
cargo run -p xtask -- register-tdx \
  --verifier 0x<TdxVerifier proxy>  \
  --rpc http://<L1 RPC>             \
  --private-key 0x<any funded key>  \
  --reth-tdx-url http://<VM_IP>:8080 \
  --register
```

---

On success the xtask prints a summary banner:

```
=======================================
  TDX PROVER REGISTERED
  AzureTdxVerifier:   0x36C02d...
  Instance address:   0xbc9d08...
  trustedParamsIndex: 0
  Block:              51
  Tx:                 0xb15f44...
=======================================
```

### Dry-run (preview without broadcasting)

Add `--dry-run` to any of the above:

```bash
cargo run -p xtask -- register-tdx \
  --verifier 0x... --rpc http://... --private-key 0x... \
  --reth-tdx-url http://... \
  --trust --register --dry-run
```

### Multi-asset release

If a release contains assets for multiple chains, use `--release-asset`:

```bash
  --release-url https://github.com/NethermindEth/nethermind-tdx/releases/tag/<TAG> \
  --release-asset taiko-tdx-prover-dev_2026-05-29
```

`--release-asset` is matched as a substring against asset names.

## All flags

| Flag | Env var | Default | Description |
|------|---------|---------|-------------|
| `--verifier` | `TDX_VERIFIER` | — | `TdxVerifier` proxy address |
| `--rpc` | `TDX_RPC` | — | L1 RPC URL |
| `--private-key` | `PRIVATE_KEY` | — | Hex key (with or without `0x`). Owner for `--trust`, any funded key for `--register` alone |
| `--reth-tdx-url` | `TDX_RETH_URL` | — | Fetch bootstrap from `GET <url>/bootstrap` (on the `reth-tdx` HTTP server inside the TDX VM). If unset, reads `~/.config/reth-tdx/bootstrap.json` |
| `--trusted-params-index` | — | `0` | Slot to read/write |
| `--pcr-bitmap` | — | `0xBA10` | 24-bit mask of PCR indices to include in trusted params (PCRs 4, 9, 11, 12, 13, 15 — Azure paravisor reference set). Overridden by the release's bitmap when `--release-url` is set |
| `--release-url` | `TDX_RELEASE_URL` | — | GitHub release page URL or direct `*.measurements.json` URL. When set, downloads the release-blessed measurements, uses its PCR bitmap, and cross-checks the live VM's PCR digests against it before broadcasting |
| `--release-asset` | `TDX_RELEASE_ASSET` | — | Substring filter when the release page has multiple `*.measurements.json` assets. Required when more than one matches |
| `--trust` | — | `false` | Call `setTrustedParams` (owner-only). Extracts `mrSeam`, `mrTd`, `teeTcbSvn`, and PCRs from the live VM's attestation quote |
| `--register` | — | `true` if no flags given | Call `registerInstance`. Submits the live VM's attestation for on-chain verification |
| `--dry-run` | — | `false` | Print transactions without broadcasting |

## Where the bootstrap comes from

`reth-tdx` (running inside the TDX VM — see
[`nethermind-tdx/reth-tdx`](https://github.com/NethermindEth/nethermind-tdx/tree/main/reth-tdx))
bootstraps eagerly on first boot. It asks the `tdxs` daemon for a fresh attestation quote
bound to a freshly-generated secp256k1 signing key, writes the record to
`~/.config/reth-tdx/bootstrap.json`, and exposes the same payload at
`GET /bootstrap`. The bootstrap is sticky — `reth-tdx` reuses the same TDX-sealed
key across restarts, so the registered instance survives a process restart or a
`reth-tdx` binary update on the same image.

raiko2 itself no longer holds a TDX bootstrap key. When `raiko2` is configured for the
`tdx/remote` route, it forwards proof requests over HTTP to the `reth-tdx` instance running
inside the VM — see
[`raiko2-prover::reth_tdx`](../crates/prover/src/reth_tdx/mod.rs) for the wire protocol.

If the underlying image changes (rebuilt mkosi image, new kernel, new init), the seal context
changes and the bootstrap key changes too — you need a fresh `--register` (and a fresh
`--trust` if `mrTd` / `mrSeam` / PCRs changed).

## Troubleshooting

### `TDX_INVALID_TRUSTED_PARAMS()` (`0xff79a3c9`) on `registerInstance`

Trusted params slot is empty. Run with `--trust` first (as owner).

### `TcbEvalExpiredOrNotFound(TcbId=1)` (`0xa78bf21a`) on `registerInstance`

The PCCS doesn't have the current TCB evaluation data loaded. On custom devnets this means
`setup_tdx_pccs_extras.sh` wasn't run after `deploy_automata_dcap.sh`. See the
[deployment guide](https://github.com/taikoxyz/taiko-mono/blob/main/packages/protocol/docs/tdx_deployment.md#step-4--deploy-the-smart-contracts).

### `TDX_ALREADY_ATTESTED()` (`0x13c26299`)

The bootstrap key is already in the registry. Re-registering the same address is blocked by
`addressRegistered[addr]`. Either rotate the prover key (rebuild the image, or wipe the
sealed key store), or wait 365 days for the existing entry to expire and re-register the
same key.

### `TDX_INVALID_PROOF()` (`0xd646c2c4`) when calling `verifyProof`

The signer recovered from the proof's ECDSA signature doesn't match the instance encoded in
the proof bytes, **for the given `_commitmentHash`**. Usually one of:

- Wrong `_commitmentHash` passed in — it must be the **pre-hash** of `LibPublicInput`
  (i.e. `hash_shasta_subproof_input(proof_carry_data)`), not the already-hashed
  `signing_hash` that raiko2 returns as the `input` field of `/v3/proof/batch/shasta`.
- `TdxVerifier`'s `taikoChainId` (set at construction) doesn't match the L2 chain the
  prover was configured against (`reth-tdx`'s `--l2-chain-id` env var).
- The verifier address the prover signed for doesn't match the contract address being
  called. `reth-tdx` reads its verifier address from the `--verifier` flag /
  `SHASTA_VERIFIER` env var at startup; raiko2 also validates this on the request side
  in its `prover.tdx` config.

### `TDX_INVALID_INSTANCE()` (`0x07b8ce1e`) when calling `verifyProof`

The recovered signer is valid as a signature, but the address isn't a registered live
instance at the given `id`. Re-run `--register` for this prover, or check that the proof's
4-byte `instance_id` prefix matches the slot you registered into.
