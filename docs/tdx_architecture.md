# raiko2 ↔ nethermind-tdx architecture

This doc explains how the raiko2 TDX prover is packaged into a sealed Linux image by the
[`nethermind-tdx`](https://github.com/NethermindEth/nethermind-tdx) repo, what runs inside
that image at runtime, and how the parts talk to each other to produce a TDX-attested
proof that lands on-chain.

For the on-chain registration flow (`AzureTdxVerifier`, `setTrustedParams`, `registerInstance`)
see [`tdx_register.md`](./tdx_register.md). For the full deployment pipeline (image build →
smart-contract deploy → registration) see
[taiko-mono — TDX deployment](https://github.com/taikoxyz/taiko-mono/blob/main/packages/protocol/docs/tdx_deployment.md).

## Repo relationships (build time vs. runtime)

```mermaid
flowchart LR
    subgraph SRC[Source repos]
        R2[raiko2<br/>--features tdx]
        TM[taiko-mono / taiko-client]
        NM[nethermind<br/>L2 execution client]
        TDXS[tdxs<br/>attestation daemon]
    end

    NMTDX[nethermind-tdx<br/>mkosi image builder]

    subgraph IMG[Sealed TDX image]
        IMG_R2[raiko2 binary]
        IMG_TC[taiko-client binary]
        IMG_NM[nethermind binary]
        IMG_TDXS[tdxs binary]
    end

    VM[Running TDX VM<br/>Azure / GCP / bare-metal qemu]

    R2 -->|cargo build| NMTDX
    TM -->|go build| NMTDX
    NM -->|dotnet build| NMTDX
    TDXS -->|go build| NMTDX
    NMTDX -->|mkosi --profile=taiko| IMG
    IMG_R2 -.runs in.-> VM
    IMG_TC -.runs in.-> VM
    IMG_NM -.runs in.-> VM
    IMG_TDXS -.runs in.-> VM
```

`nethermind-tdx` is **not** an executable — it's a build system (mkosi + nix) that pins
versions of each upstream repo in [`taiko-tdx-prover/mkosi.build`](https://github.com/NethermindEth/nethermind-tdx/blob/master/taiko-tdx-prover/mkosi.build)
and produces a single, measured, reproducible EFI image. The TDX measurements (`mrTd`,
PCRs) cover the entire image, so swapping any binary inside requires re-attesting on-chain.

## Inside the image (runtime topology)

```mermaid
flowchart TB
    subgraph EXT[Outside the VM]
        OP[Operator<br/>SSH + xtask register-tdx]
        TC_OFF[taiko-client off-chain<br/>proof requestor]
        L1[L1 RPC + Beacon]
        ETH[Ethereum L1<br/>Inbox + TdxAndZkVerifier]
    end

    subgraph VM[TDX VM image]
        direction TB
        RTI[runtime-init<br/>oneshot:<br/>SSH provisioning,<br/>secret unsealing]
        TDXS[tdxs<br/>attestation daemon<br/>/var/tdxs.sock]
        NMSURGE[nethermind-surge<br/>L2 execution client<br/>:L2_HTTP_PORT]
        TAIKO_CLI[taiko-client<br/>L2 sync driver]
        RAIKO2[raiko2<br/>TDX prover<br/>+ HTTP API]
    end

    OP -->|"port 22/8080: SSH key inject, /guest_data"| RTI
    L1 -->|L1 blocks + blobs| TAIKO_CLI
    L1 -->|L1 RPC / beacon| RAIKO2
    TAIKO_CLI -->|"engine API (JWT)"| NMSURGE
    RAIKO2 -->|"JSON-RPC: debug + eth"| NMSURGE
    RAIKO2 -->|"Unix socket: issue/metadata/verify quote"| TDXS
    TC_OFF -->|"HTTP /v3/proof/batch/shasta"| RAIKO2
    TC_OFF -->|signed prove tx| ETH

    classDef oneshot fill:#f5f5f5,stroke:#999,stroke-dasharray: 3 3
    class RTI oneshot
```

| Service             | Source repo            | Role inside the image                                                                                  |
| ------------------- | ---------------------- | ------------------------------------------------------------------------------------------------------ |
| `runtime-init`      | `nethermind-tdx/init`  | oneshot: configures network, accepts an SSH pubkey via port 8080, unseals persistent storage           |
| `tdxs`              | `NethermindEth/tdxs`   | Long-running daemon that owns quote generation. Listens on `/var/tdxs.sock` (group `tdx`). Issues Intel TDX DCAP quotes bound to caller-supplied 32-byte user-data. |
| `nethermind-surge`  | `NethermindEth/nethermind` | L2 execution client. Provides the engine API to `taiko-client` and JSON-RPC (incl. `debug_*`) to `raiko2` for witness fetching. |
| `taiko-client`      | `taikoxyz/taiko-mono`  | L2 sync driver — watches L1 Inbox events and feeds blocks to nethermind via the engine API.            |
| `raiko2`            | `taikoxyz/raiko2`      | TDX prover. Built with `--features tdx`, exposes the proof HTTP API, and signs proofs with a key derived inside the TEE. |

## Bootstrap: how raiko2 gets a TDX-bound key

The bootstrap happens **once** per image instance, on first start of `raiko2.service`. It's
what makes a fresh VM into something an operator can register on-chain.

```mermaid
sequenceDiagram
    participant Sys as systemd
    participant R2 as raiko2 (--features tdx)
    participant TDXS as tdxs daemon
    participant Disk as /home/raiko2/.config/raiko2/tdx/
    participant Op as Operator (xtask)
    participant L1 as L1 chain
    participant TV as AzureTdxVerifier

    Sys->>R2: start raiko2.service
    R2->>R2: TdxProver::ensure_bootstrapped()
    Note over R2: first boot only
    R2->>R2: generate fresh secp256k1 keypair
    R2->>Disk: write secrets/priv.key (0600)
    R2->>TDXS: issue_attestation(user_data=address‖0x00..., nonce=random 32B)
    TDXS-->>R2: TDX quote (Intel DCAP) + Azure vTPM metadata
    R2->>Disk: write bootstrap.json (camelCase)

    Op->>R2: GET /v3/proof/tdx/bootstrap
    R2-->>Op: { issuerType, publicKey, quote, nonce, metadata }
    Op->>L1: cargo xtask register-tdx --register
    L1->>TV: registerInstance(idx, attestation)
    Note over TV: on-chain DCAP verify<br/>+ trusted-params equality
    TV-->>L1: instance admitted
    Op->>R2: ready to accept /v3/proof/* requests
```

A few invariants worth remembering:

- The private key is generated **inside the TEE** and never leaves it. Its public Ethereum
  address is embedded in the quote's user-data, so the on-chain registry can trust that
  signatures from that address came from this exact image build.
- The bootstrap file is sticky across `raiko2` restarts (key + quote survive on the
  encrypted persistent disk), so registering once is enough until image measurements
  change.
- Rebuilding the image changes `mrTd` / PCRs → the existing trusted-params slot no longer
  matches → operators must run `--trust` against a fresh slot before re-registering.
  See [`tdx_register.md`](./tdx_register.md#when-do-i-need---trust).

## Proof generation flow

A proposal proof request from `taiko-client` flows through every component in the image:

```mermaid
sequenceDiagram
    participant TC as taiko-client (off-chain)
    participant R2 as raiko2
    participant NM as nethermind-surge
    participant L1 as L1 RPC + beacon
    participant TDXS as tdxs
    participant ETH as Inbox + TdxAndZkVerifier

    TC->>R2: POST /v3/proof/batch/shasta<br/>{ proof_type: "tdx", proposals }
    R2->>L1: fetch L1 headers + blobs<br/>(proposal event, anchors)
    R2->>NM: debug_executionWitness<br/>+ eth_getBlock for each L2 block
    NM-->>R2: blocks + witnesses
    R2->>R2: build ShastaCommitment<br/>+ signing_hash (= shasta_aggregation_output)
    R2->>R2: ECDSA-sign signing_hash<br/>with TDX-sealed key
    R2->>TDXS: issue_attestation(user_data=signing_hash, nonce=...)
    TDXS-->>R2: fresh quote over signing_hash
    R2-->>TC: { proof: 0x<instance_id‖addr‖sig>,<br/>quote, input: signing_hash, extra_data }

    Note over TC: pairs TDX proof with a ZK proof<br/>(RISC0 or SP1) for TdxAndZkVerifier

    TC->>ETH: prove(input, subProofs=[ZK, TDX])
    Note over ETH: ComposeVerifier iterates in<br/>ascending VerifierID order<br/>(RISC0=5/SP1=6, TDX_RETH=7)
    ETH->>ETH: AzureTdxVerifier.verifyProof:<br/>ECDSA.recover == registered instance
    ETH-->>TC: proven ✓
```

Key wire-format facts:

- The 89-byte TDX proof body (`instance_id(4) ‖ address(20) ‖ signature(65)`) is
  byte-identical to the existing SGX wire format, so the on-chain verifier reuses the same
  recover-and-compare logic.
- `proof.input` returned to `taiko-client` is `signing_hash =
  shasta_aggregation_output(commitment, chain_id, verifier, tdx_instance)` — what the
  ECDSA signature is taken over, not the carry-data hash. This matters because the same
  `proof.input` is what the on-chain `Inbox` will reconstruct when calling `verifyProof`.
- The Intel TDX quote returned alongside the proof is **not** included in the on-chain
  submission for steady-state proving — it is only consumed by `registerInstance` at
  enrollment time. Per-proof attestation is implicit: the on-chain instance registry
  proves the signing key was produced by an attested image.

## Where each piece is configured

| What                                  | Where it lives                                                                                            |
| ------------------------------------- | --------------------------------------------------------------------------------------------------------- |
| Image build pin (raiko2 git ref)      | `nethermind-tdx/taiko-tdx-prover/mkosi.build` — `RAIKO2_VERSION` / `RAIKO2_GIT_URL`                       |
| Per-deployment env (chain ids, RPCs)  | `nethermind-tdx/env.json` (templated into `/etc/nethermind-surge/.env` at build time)                     |
| raiko2 runtime config                 | `/etc/nethermind-surge/raiko2_config.toml` inside the VM (rendered from `env.json` + `raiko2_config.toml.in`) |
| tdxs socket path used by raiko2       | `prover.tdx.socket_path` in raiko2 config — defaults to `/var/tdxs.sock` and matches `tdxs.socket`        |
| On-chain verifier address (per L2)    | raiko2 chain spec `verifier_address_forks.TDX`                                                            |
| Bootstrap data (TDX-sealed key+quote) | `/home/raiko2/.config/raiko2/tdx/{secrets/priv.key, bootstrap.json}` (encrypted persistent disk)          |

If you change anything in the first column you must rebuild the image; the resulting
`mrTd` will differ and the on-chain trusted-params slot will need to be re-issued before
new instances can register.
