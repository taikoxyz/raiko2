# Explicit Prover Route Registration Design

## Problem

The server currently treats `prover.guest_system` and `prover.runner` as one global route. That
single route also controls which pipelines are registered. This creates implicit behavior that is
not represented by configuration. In particular, a production host configured as
`risc0/network` also registers SP1, while a host configured as `sgx/remote` registers SGX lanes and
returns before registering any ZK pipeline.

The HTTP API already selects a concrete `proof_type` per request. Server capabilities should follow
the same model: each concrete proof type must be enabled explicitly, and no enabled proof type
should imply another one.

## Configuration

Replace the global `guest_system` and `runner` fields with an explicit route table:

```toml
[prover.routes]
risc0 = "network"
sp1 = "network"
sgx = "remote"
sgxgeth = "remote"
```

Supported entries are:

| Proof type | Allowed runner |
| --- | --- |
| `risc0` | `local`, `network` |
| `sp1` | `local`, `network` |
| `native` | `local` |
| `sgx` | `remote` |
| `sgxgeth` | `remote` |

An omitted entry is disabled. At least one route must be enabled. Backend parameter sections such
as `[prover.risc0]`, `[prover.sp1]`, `[prover.boundless]`, and `[prover.remote_sgx]` do not enable a
route by themselves.

This is an intentional breaking configuration change. The old `prover.guest_system`,
`prover.runner`, `--prover`, and `RAIKO2_PROVER` inputs are removed rather than retained as a second
source of truth. A full route-table override is available through `--prover-routes` or
`RAIKO2_PROVER_ROUTES` using a comma-separated value:

```text
risc0/network,sp1/network,sgx/remote,sgxgeth/remote
```

The override replaces the configured route table as one atomic value.

## Validation

Configuration validation fails at startup when:

- no route is enabled,
- a proof type uses an unsupported runner,
- `risc0/network` is enabled without valid Boundless configuration,
- `sgx/remote` is enabled without `prover.remote_sgx.base_url`,
- `sgxgeth/remote` is enabled without `prover.remote_sgx.sgxgeth_base_url`,
- either SGX lane is enabled with a zero remote timeout,
- a `zk_any` target names a ZK backend that is not enabled, or
- a route requires a prover implementation excluded from the compiled binary.

Backend-specific validation should only require credentials and endpoints for enabled routes. A
disabled backend's unused settings must not prevent startup.

## Pipeline Registration And Request Routing

Pipeline registration iterates over the explicit route table. Each enabled concrete proof type
registers exactly one matching pipeline:

- `risc0/local` -> local RISC0,
- `risc0/network` -> Boundless,
- `sp1/local|network` -> SP1 with the selected prover mode,
- `native/local` -> native execution,
- `sgx/remote` -> the raiko2 SGX remote,
- `sgxgeth/remote` -> the gaiko2 remote.

The production `host` feature can therefore register network ZK and remote SGX pipelines in the
same process without compiling local prover implementations. The `local-provers` feature can
register local routes when explicitly requested, but no longer registers every local pipeline by
default.

V4 proposal and aggregation requests continue carrying one concrete `proof_type`. Route selection
looks up that proof type in the configured table. A missing entry returns
`unsupported_proof_type`. There is no change to request schemas, proof artifacts, guest inputs,
guest ELF files, or public inputs.

`zk_any` remains an admission policy rather than a backend. It may select only enabled `sp1` or
`risc0` routes.

## Readiness And Observability

Readiness checks only the capabilities represented by enabled routes. Startup logs expose the full
sanitized route set instead of one default route, and include remote URLs only for enabled SGX
lanes. Existing per-request route and proof-type metrics remain unchanged.

## Migration Sample

A host serving both hosted ZK systems and both SGX lanes uses:

```toml
[prover.routes]
risc0 = "network"
sp1 = "network"
sgx = "remote"
sgxgeth = "remote"

[prover.sp1]
mode = "prove"
prover = "network"
recursion = "plonk"
verify = true
network_mode = "reserved"
fulfillment_strategy = "reserved"
skip_simulation = true
cycle_limit = 1000000000000
timeout_secs = 7200

[prover.remote_sgx]
base_url = "http://sgx-prover:8080"
sgxgeth_base_url = "http://sgxgeth-prover:8080"
timeout_ms = 300000
```

The Boundless settings remain under `[prover.boundless]`. Production credentials must continue to
come from deployment configuration or secrets rather than checked-in examples.

## Verification

Tests must cover:

- strict parsing and validation of every allowed and rejected route,
- rejection of old global route fields,
- atomic CLI/environment route-table replacement,
- independent registration of RISC0 and SP1,
- simultaneous network ZK and remote SGX registration in a host-only build,
- disabled proof types returning `unsupported_proof_type`,
- `zk_any` rejecting disabled targets,
- readiness checking only enabled capabilities,
- startup summary output for multiple routes, and
- proposal and aggregation routing for each enabled proof type.

Documentation and checked-in Docker examples must use the new route table. No generated guest ELF
artifact is rebuilt because the change is host-only.
