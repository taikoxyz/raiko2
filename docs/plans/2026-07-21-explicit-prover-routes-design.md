# Self-Contained Prover Configuration Design

## Problem

The server must independently enable RISC0, SP1, native execution, SGX, and SGXGETH. The first
version of this change introduced a separate `[prover.routes]` table, but that left an operator to
discover hidden relationships such as `sgx = "remote"` requiring `[prover.remote_sgx]`, or
`risc0 = "network"` requiring `[prover.boundless]`.

Enablement, execution selection, and backend parameters should be colocated. A proof type must not
be enabled by an unrelated section, and there must be no second file-level route table to keep in
sync.

## Configuration Model

Each concrete proof type owns one self-contained table with an explicit `enabled` field. The table
also contains the setting that selects its supported execution implementation:

- RISC0 uses `runner = "local" | "network"`.
- SP1 uses its existing `prover = "local" | "mock" | "network"`; local and mock map to the local
  runner, while network maps to the network runner.
- Native execution has no runner setting because its only supported runner is local.
- SGX and SGXGETH have no runner setting because their only supported runner is remote.

This intentional asymmetry avoids redundant pairs such as `runner = "network"` plus
`prover = "local"`, and avoids configurable values that can only have one valid value.

```toml
[prover.risc0]
enabled = true
runner = "network"
bonsai = true
snark = true
mock = false
execution_po2 = 20

[prover.sp1]
enabled = true
prover = "network"
mode = "prove"
recursion = "plonk"
verify = true

[prover.native]
enabled = false

[prover.sgx]
enabled = true
base_url = "http://sgx-prover:8080"
timeout_ms = 300000

[prover.sgxgeth]
enabled = true
base_url = "http://sgxgeth-prover:8080"
timeout_ms = 300000
```

At least one proof type must be enabled. Disabled sections are not validated for credentials or
connectivity. `ProverConfig` remains the sole code-level owner of route resolution through
`runner(proof_type)`, `is_enabled(proof_type)`, and stable enabled-route iteration helpers.

## Boundless

Boundless is the network implementation of RISC0, so its entire global configuration is nested
under `[prover.risc0.boundless]` rather than represented as a peer prover:

```toml
[prover.risc0.boundless]
offchain = false
rpc_url = "https://base-rpc.example.com"
signer_key = "configured-by-secret-store"
poll_interval_ms = 10000
timeout_ms = 3600000
rebid_timeout_ms = 300000
rebid_price_step_bps = 5000
rebid_max_attempts = 4

[prover.risc0.boundless.deployment]
deployment_type = "base"

[prover.risc0.boundless.deployment.overrides]
order_stream_url = "https://base-mainnet.boundless.network"

[prover.risc0.boundless.batch_quote]
strategy = "raiko_agent"

[prover.risc0.boundless.aggregation_quote]
strategy = "raiko_agent"

[prover.risc0.boundless.offer_params.batch]
pricing_mode = "market"

[prover.risc0.boundless.offer_params.batch.timeouts]
lock_timeout = 120

[prover.risc0.boundless.offer_params.aggregation]
pricing_mode = "market"

[prover.risc0.boundless.offer_params.aggregation.timeouts]
lock_timeout = 120
```

Every existing Boundless field and nested table keeps the same semantics. Only its global TOML path
changes. Pair-specific overrides remain under each `[[rpc.pairs]]` entry as `boundless = {...}` or
the equivalent nested pair table because they belong to network-pair selection. The effective
configuration remains: global `prover.risc0.boundless` defaults, then the selected pair override.

Boundless credentials and offer validation run only when `prover.risc0.enabled = true` and
`prover.risc0.runner = "network"`. Local or disabled RISC0 must not require them.

## Overrides

`--prover-routes` and `RAIKO2_PROVER_ROUTES` remain optional operational overrides. They atomically
replace enabled proof types and update the execution selector inside the corresponding self-contained
table; they are not a second file-level configuration section. Omitted proof types are disabled.
Backend parameters remain in their proof-type table or existing lane-specific CLI/environment
overrides.

The legacy file sections `[prover.routes]`, `[prover.boundless]`, and `[prover.remote_sgx]` are
rejected. The pre-PR global `prover.guest_system`, `prover.runner`, `--prover`, and `RAIKO2_PROVER`
inputs also remain rejected.

## Runtime Behavior

Request routing, pipeline registration, readiness, resource preparation, and startup summaries all
consume `ProverConfig` route helpers. A disabled proof type returns `unsupported_proof_type`. No
backend table implicitly enables another proof type.

SGX and SGXGETH keep distinct public route identities (`sgx/remote` and `sgxgeth/remote`). Legacy
persisted SGXGETH records using `sgx/remote` retain the compatibility and fail-closed startup
migration already defined by this branch.

## Verification

Tests must cover:

- parsing a combined self-contained production configuration,
- rejection of the superseded standalone route/backend tables,
- every proof type's enablement and runner derivation,
- atomic CLI/environment route overrides,
- disabled backend validation skipping,
- the complete Boundless global configuration and every nested child table at its new path,
- pair-specific Boundless overrides applied after the nested global configuration,
- RISC0 local mode ignoring unused Boundless credentials,
- independent request routing and pipeline registration,
- readiness and startup summaries for enabled lanes only, and
- Docker/sample/API/operations documentation using only the self-contained model.

This is host-only. Guest inputs, proof formats, guest ELF files, image IDs, public inputs, and
on-chain verification do not change.
