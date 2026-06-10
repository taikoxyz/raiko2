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
For VM deployment on Azure and GCP see the
[nethermind-tdx README](https://github.com/NethermindEth/nethermind-tdx/blob/main/README.md).

> **Two verifier flavours — the xtask auto-detects which.** The flow it runs is
> chosen from the prover's bootstrap `issuer_type`:
> - **`azure`** → the Azure vTPM flow against **`AzureTdxVerifier`**. The Azure
>   paravisor binds the TDX quote into a vTPM quote; measurements are vTPM **PCRs**
>   (`--pcr-bitmap`, default `0xBA10`).
> - **`tdx`** (GCP Confidential VMs, bare-metal TDX) → the native-DCAP flow against
>   **`GcpTdxVerifier`**
>   ([`taiko-mono/.../GcpTdxVerifier.sol`](https://github.com/taikoxyz/taiko-mono/blob/main/packages/protocol/contracts/layer1/verifiers/GcpTdxVerifier.sol)).
>   The prover emits a raw Intel TDX DCAP quote (read from `/dev/tdx_guest` via
>   configfs-tsm); measurements are the quote's **RTMR0..3** (`--rtmr-mask`,
>   default `0b0111` = RTMR0,1,2). No vTPM.
>
> `--verifier` must point at the matching contract for the prover's issuer. The
> image's issuer is fixed by its build profile (`make build AZURE=true` → azure,
> `GCP=true` / bare-metal → tdx), not by env.json.

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

### 3. Native TDX (GCP Confidential VM / bare-metal)

Identical command shape — the xtask detects `issuer_type == "tdx"` from the
bootstrap and automatically uses the **`GcpTdxVerifier`** native-DCAP flow
(raw quote + RTMRs, no vTPM). Point `--verifier` at the deployed `GcpTdxVerifier`
proxy. Trusted params bind `teeTcbSvn`, `mrSeam`, `mrTd`, and the quote RTMRs
selected by `--rtmr-mask`.

```bash
cargo run -p xtask -- register-tdx \
  --verifier 0x<GcpTdxVerifier proxy> \
  --rpc http://<L1 RPC>               \
  --private-key 0x<owner key>         \
  --reth-tdx-url http://<VM_IP>:8080  \
  --rtmr-mask 0b0111                  \
  --trust --register
```

Under the hood the native path unwraps tdxs's attestation document
(`hex(JSON {RawQuote, UserData})`) to the raw DCAP quote, parses
`teeTcbSvn`/`mrSeam`/`mrTd`/`RTMR0-3` from it for `setTrustedParams`, and submits
`registerInstance(idx, rawQuote, userData, nonce)` — the prover address is bound
on-chain via `reportData == sha256(userData || nonce)`.

> The `GcpTdxVerifier` must be deployed against the **same Automata DCAP** the
> Azure verifier uses (see `DeployGcpTdxVerifier.s.sol`). For native TDX,
> `--release-url` should point to a `*.gcp_measurements.json` asset. The xtask
> cross-checks every release field it contains (`tee_tcb_svn`/`teeTcbSvn`,
> `mr_seam`/`mrSeam`, `mr_td`/`mrTd`, `rtmr0..rtmr3`) against the live quote
> before broadcasting.

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
| `--pcr-bitmap` | — | `0xBA10` | **Azure issuer only.** 24-bit mask of PCR indices to include in trusted params (PCRs 4, 9, 11, 12, 13, 15 — Azure paravisor reference set). Overridden by the release's bitmap when `--release-url` is set |
| `--rtmr-mask` | — | `0b0111` | **Native (tdx) issuer only.** 4-bit mask of RTMRs to bind into `GcpTdxVerifier` trusted params. Default = RTMR0,1,2 (image/boot measurements); RTMR3 is excluded because it is typically extended by the runtime |
| `--release-url` | `TDX_RELEASE_URL` | — | GitHub release page URL or direct measurements asset URL. Azure uses `*.measurements.json` for PCRs; native TDX uses `*.gcp_measurements.json` for `teeTcbSvn`/`mrSeam`/`mrTd`/RTMR cross-checks before broadcasting |
| `--release-asset` | `TDX_RELEASE_ASSET` | — | Substring filter when the release page has multiple matching measurement assets. Required when more than one matches |
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
`tdx_dcap/remote` route, it forwards proof requests over HTTP to the `reth-tdx` instance running
inside the VM — see
[`raiko2-prover::reth_tdx`](../crates/prover/src/reth_tdx/mod.rs) for the wire protocol.

If the underlying image changes (rebuilt mkosi image, new kernel, new init), the seal context
changes and the bootstrap key changes too — you need a fresh `--register` (and a fresh
`--trust` if `mrTd` / `mrSeam` / PCRs changed).

## Verifying a proof on-chain

Once an instance is registered you can confirm a real proof verifies against the verifier
contract. `IProofVerifier.verifyProof(uint256 _proposalAge, bytes32 _commitmentHash, bytes _proof)`
is a **`view`** function that **reverts** on an invalid proof — so a non-reverting `cast call`
means "verified". This is identical for `AzureTdxVerifier` and `GcpTdxVerifier`.

The catch is computing `_commitmentHash`. The proof response (`/v3/proof/batch/shasta`) gives:
- `data.proof.proof` — the 89-byte proof: `instance_id(4) || instance(20) || signature(65)`.
- `data.proof.input` — the **already-signed** hash. Do **not** pass this to `verifyProof`.
- `data.proof.extra_data.shasta.proof_carry_data` (single) or `proof_carry_data_vec` (aggregate).

`reth-tdx` (in `sign_proposal` / `sign_aggregation`, even for one proposal) signs:

```
input = hash_public_input( hash_commitment(Commitment), chainId, verifier, instance )
```

so `_commitmentHash = hash_commitment(Commitment)`, where
`Commitment = build_shasta_commitment_from_proof_carry_data_vec(proof_carry_data_vec)`:

| Commitment field | from carry |
|---|---|
| `firstProposalId` | `carry[0].transition_input.proposal_id` |
| `firstProposalParentBlockHash` | `carry[0].transition_input.parent_block_hash` |
| `lastProposalHash` | `carry[last].transition_input.proposal_hash` |
| `actualProver` | `carry[0].transition_input.actual_prover` |
| `endBlockNumber` / `endStateRoot` | `carry[last].transition_input.checkpoint.{blockNumber,stateRoot}` |
| `transitions[i]` | `{proposer, timestamp, checkpoint.blockHash}` from `carry[i]` |

`hash_commitment` = `keccak256` over the Solidity buffer (32-byte words):
`0x20, firstProposalId, firstProposalParentBlockHash, lastProposalHash, actualProver,
endBlockNumber, endStateRoot, 0xe0, transitionsLen, then (proposer, timestamp, blockHash)×N`.
`hash_public_input(C, chainId, verifier, instance)` =
`keccak256(bytes32("VERIFY_PROOF") || uint256(chainId) || uint160(verifier) || C || uint160(instance))`.

> Note: `_commitmentHash` is **not** `hash_shasta_subproof_input` — that's the SGX/zk
> single-proof hash. The reth-tdx TDX verifier always uses the **Commitment** (aggregation)
> hash above. Source: `crates/primitives-shasta/src/instance.rs`
> (`shasta_aggregation_output`, `build_shasta_commitment_from_proof_carry_data_vec`).

You usually don't need to do this by hand — the **`verify-tdx-proof`** skill fetches the proof,
computes `_commitmentHash`, and calls `verifyProof` for you (Azure or GCP):

```bash
~/.claude/skills/verify-tdx-proof/verify.sh \
  --verifier 0x<verifier proxy> --rpc <L1 RPC> --raiko-url <raiko2 base url> \
  --proposal-id <N> --l1 <l1 inclusion block> --l2 <l2 block> --anchor <last anchor block> [--aggregate]
```

It prints `✅ VERIFIED` or `❌ verifyProof REVERTED` (with the revert selector).

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

- Wrong `_commitmentHash` passed in. It is **`hash_commitment(Commitment)`** — the
  pre-hash that `LibPublicInput.hashPublicInputs` wraps — **not** the already-hashed
  `input` field that `/v3/proof/batch/shasta` returns (that one is the signed hash, i.e.
  `hashPublicInputs(commitment, …)`, and passing it makes `ecrecover` resolve to a junk
  address). It is also **not** `hash_shasta_subproof_input` — that's the SGX/zk
  single-proof path; the reth-tdx TDX verifier always uses the **Commitment** hash (built
  from `proof_carry_data[_vec]` via `build_shasta_commitment_from_proof_carry_data_vec`),
  even for a single proposal. See [Verifying a proof on-chain](#verifying-a-proof-on-chain).
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
